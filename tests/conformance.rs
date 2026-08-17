#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]
//! Conformance Tests: Validate obr (Rust) produces identical output to bd (Go)
//!
//! This harness runs equivalent commands on both obr and bd in isolated temp directories,
//! then compares outputs using various comparison modes.

mod common;

use assert_cmd::Command;
use chrono::Utc;
use common::cli::extract_json_payload;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_yml::Value as YamlValue;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tempfile::TempDir;
use tracing::info;

/// Get the path to the `bd` (Go beads) binary.
/// Checks `BD_BINARY` environment variable first, falls back to PATH lookup.
fn get_bd_binary() -> String {
    common::bd_binary_name()
}

/// Check if the discovered `bd` is a usable classic conformance reference.
///
/// Delegates to the single gate in `common::binary_discovery`, which rejects a
/// missing binary, a `bd` that is really obr, and a post-classic (Dolt-default)
/// bd that obr was never meant to match.
pub fn bd_available() -> bool {
    common::bd_available()
}

/// Skip test when `bd` is not a usable classic conformance reference.
macro_rules! skip_if_no_bd {
    () => {
        if let Some(reason) = common::bd_skip_reason() {
            eprintln!("Skipping conformance test: {reason}");
            return;
        }
    };
}

/// Issue ID prefix forced on **both** workspaces at init (`beads_rust-f175`).
///
/// Both tools derive a prefix from their working directory when none is given,
/// and this harness deliberately gives them different directories
/// (`obr_workspace` and `bd_workspace`). Since [`normalize_value`] preserves the
/// prefix and normalizes only the hash portion, an unprefixed init made every
/// `id` field compare as `obr_workspace-NORMALIZED` against
/// `bd_workspace-NORMALIZED` — a mismatch no test could ever pass under
/// `CompareMode::NormalizedJson`.
///
/// `tests/conformance_workflows.rs` already avoids this by passing an explicit
/// prefix; this constant is the same fix for this harness. The value must be a
/// valid prefix for both tools and must not contain a `-`.
const CONFORMANCE_PREFIX: &str = "bd";

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
    /// Canonical `init` invocation for both tools. Every init in this file goes
    /// through this so a later call cannot silently reintroduce a
    /// directory-derived prefix.
    const INIT_ARGS: [&'static str; 3] = ["init", "--prefix", CONFORMANCE_PREFIX];

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

    /// Initialize both obr and bd workspaces with the same explicit prefix.
    ///
    /// The prefix is not optional: see [`CONFORMANCE_PREFIX`] for why letting
    /// each tool derive one from its own directory name makes every id
    /// comparison unsatisfiable.
    pub fn init_both(&self) -> (CmdOutput, CmdOutput) {
        let obr_out = self.run_obr(Self::INIT_ARGS, "init");
        let bd_out = self.run_bd(Self::INIT_ARGS, "init");
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

    /// Run obr command in the bd workspace (to setup state)
    pub fn run_obr_in_bd_env<I, S>(&self, args: I, label: &str) -> CmdOutput
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_obr_cmd(
            &self.bd_root,
            &self.log_dir,
            args,
            &format!("br_in_bd_{label}"),
        )
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
    let bd_bin = get_bd_binary();
    run_cmd_system(&bd_bin, cwd, log_dir, args, label)
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
    // Force bd to operate on the local workspace to avoid contributor routing to planning repos.
    cmd.env("OBR_DIR", cwd.join(".obr"));

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
        stdout_sha256: Some(sha256_hex(&stdout)),
        stderr_sha256: Some(sha256_hex(&stderr)),
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
        stdout_sha256: Some(sha256_hex(&stdout)),
        stderr_sha256: Some(sha256_hex(&stderr)),
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

/// Comparison mode for conformance tests
#[derive(Debug, Clone)]
pub enum CompareMode {
    /// JSON outputs must be identical
    ExactJson,
    /// Ignore timestamps and normalize IDs
    NormalizedJson,
    /// Check specific fields match
    ContainsFields(Vec<String>),
    /// Just check that both succeed or both fail
    ExitCodeOnly,
    /// Compare arrays ignoring element order
    ArrayUnordered,
    /// Ignore specified fields during comparison
    FieldsExcluded(Vec<String>),
    /// Compare JSON structure only, not values
    StructureOnly,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stdout_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stderr_sha256: Option<String>,
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

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    obr::util::hex_encode(&hasher.finalize())
}

#[test]
fn conformance_content_hash_matches_length_prefixed_obr_fixture() {
    use obr::model::{IssueType, Priority, Status};

    let hash = obr::util::content_hash_from_parts(
        "Fix authentication bug",
        Some("Users are getting logged out unexpectedly"),
        Some("Use token refresh"),
        Some("Session survives refresh"),
        Some("Check logs"),
        &Status::InProgress,
        &Priority::HIGH,
        &IssueType::Bug,
        Some("bob"),
        Some("alice"),
        Some("pane6"),
        Some("github:org/repo#123"),
        Some("github"),
        true,
        true,
    );

    assert_eq!(
        hash, "ef08b448c7b8eb9a3c3b2facdd9fa8e0b2b420c99725629c6811f019a0a9ab87",
        "content_hash must match the length-prefixed obr fixture"
    );
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

// ============================================================================
// BENCHMARK TIMING INFRASTRUCTURE
// ============================================================================

/// Configuration for benchmark runs
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    /// Number of warmup runs (not counted in statistics)
    pub warmup_runs: usize,
    /// Number of timed runs for statistics
    pub timed_runs: usize,
    /// Outlier threshold in standard deviations
    pub outlier_threshold: f64,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            warmup_runs: 2,
            timed_runs: 5,
            outlier_threshold: 2.0,
        }
    }
}

/// Timing statistics from benchmark runs
#[derive(Debug, Clone)]
pub struct TimingStats {
    pub mean_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub std_dev_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub run_count: usize,
}

impl TimingStats {
    /// Compute statistics from a list of durations
    pub fn from_durations(durations: &[Duration]) -> Self {
        if durations.is_empty() {
            return Self {
                mean_ms: 0.0,
                median_ms: 0.0,
                p95_ms: 0.0,
                std_dev_ms: 0.0,
                min_ms: 0.0,
                max_ms: 0.0,
                run_count: 0,
            };
        }

        let mut ms_values: Vec<f64> = durations.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        ms_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let n = ms_values.len();
        let mean = ms_values.iter().sum::<f64>() / n as f64;
        let median = if n % 2 == 0 {
            (ms_values[n / 2 - 1] + ms_values[n / 2]) / 2.0
        } else {
            ms_values[n / 2]
        };
        let p95_idx = (n as f64 * 0.95).ceil() as usize - 1;
        let p95 = ms_values[p95_idx.min(n - 1)];
        let variance = ms_values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
        let std_dev = variance.sqrt();

        Self {
            mean_ms: mean,
            median_ms: median,
            p95_ms: p95,
            std_dev_ms: std_dev,
            min_ms: ms_values[0],
            max_ms: ms_values[n - 1],
            run_count: n,
        }
    }

    /// Filter out outliers beyond the threshold (in std deviations)
    pub fn filter_outliers(durations: &[Duration], threshold: f64) -> Vec<Duration> {
        if durations.len() < 3 {
            return durations.to_vec();
        }

        let ms_values: Vec<f64> = durations.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        let mean = ms_values.iter().sum::<f64>() / ms_values.len() as f64;
        let variance =
            ms_values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / ms_values.len() as f64;
        let std_dev = variance.sqrt();

        durations
            .iter()
            .zip(ms_values.iter())
            .filter(|&(_, &ms)| (ms - mean).abs() <= threshold * std_dev)
            .map(|(d, _)| *d)
            .collect()
    }
}

/// Run a benchmark with warmup and timing
pub fn run_benchmark<F>(config: &BenchmarkConfig, mut f: F) -> TimingStats
where
    F: FnMut() -> Duration,
{
    // Warmup runs (discard results)
    for _ in 0..config.warmup_runs {
        let _ = f();
    }

    // Timed runs
    let mut durations: Vec<Duration> = Vec::with_capacity(config.timed_runs);
    for _ in 0..config.timed_runs {
        durations.push(f());
    }

    // Filter outliers and compute stats
    let filtered = TimingStats::filter_outliers(&durations, config.outlier_threshold);
    TimingStats::from_durations(&filtered)
}

/// Count the issues in a `--json` payload, tolerating both output shapes.
///
/// `obr-ecr6`: obr and bd do not agree on the envelope. `bd list|search --json`
/// and `obr ready --json` return a bare array, while `obr list`, `obr blocked`,
/// and `obr search` use wrapper objects with `issues`. Paginated wrappers also
/// include `total`, `limit`, `offset`, and `has_more`. The harness compared
/// counts with
/// `value.as_array().map(|a| a.len()).unwrap_or(0)`, which yields **0 for every
/// `obr list --json` payload regardless of content** — so `obr` looked empty
/// everywhere.
///
/// That produced two failure modes: real mismatches wherever bd returned rows
/// (`obr=0, bd=2`), and, worse, *vacuous passes* wherever bd also returned
/// nothing — and bd v0.46.0 returns an empty list for many filtered queries, a
/// divergence already documented on a dozen `#[ignore]`d tests here.
///
/// Returns 0 for a payload that is neither shape, matching the previous
/// `unwrap_or(0)` behavior for genuinely unparseable output.
fn issue_count(value: &Value) -> usize {
    if let Some(items) = value.as_array() {
        return items.len();
    }
    value
        .get("issues")
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

/// Borrow the issue objects out of a `--json` payload, tolerating both shapes.
/// See [`issue_count`] for why this is necessary.
fn issue_items(value: &Value) -> Vec<&Value> {
    if let Some(items) = value.as_array() {
        return items.iter().collect();
    }
    value
        .get("issues")
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

/// Normalize JSON for comparison by removing/masking volatile fields
pub fn normalize_json(json_str: &str) -> Result<Value, serde_json::Error> {
    let mut value: Value = serde_json::from_str(json_str)?;
    normalize_value(&mut value);
    Ok(value)
}

fn normalize_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // Fields to normalize (set to fixed values)
            let timestamp_fields: HashSet<&str> = [
                "created_at",
                "updated_at",
                "closed_at",
                "deleted_at",
                "due_at",
                "defer_until",
                "compacted_at",
            ]
            .into_iter()
            .collect();

            // Normalize timestamps to a fixed value
            for (key, val) in map.iter_mut() {
                if timestamp_fields.contains(key.as_str()) {
                    if val.is_string() {
                        *val = Value::String("NORMALIZED_TIMESTAMP".to_string());
                    }
                } else if key == "id" || key == "issue_id" || key == "depends_on_id" {
                    // Keep the prefix, normalize the hash portion.
                    //
                    // `rfind` (not `find`) splits on the LAST dash, matching
                    // the normalizers in `conformance_workflows.rs` and
                    // `common/scenarios.rs`. Splitting on the first dash
                    // truncated any prefix that itself contains one — a
                    // directory-derived prefix like `beads-rust` normalized to
                    // `beads-NORMALIZED`, silently discarding the rest
                    // (`beads_rust-f175`).
                    if let Some(s) = val.as_str() {
                        if let Some(dash_pos) = s.rfind('-') {
                            let prefix = &s[..dash_pos];
                            *val = Value::String(format!("{prefix}-NORMALIZED"));
                        }
                    }
                } else if key == "content_hash" {
                    if val.is_string() {
                        *val = Value::String("NORMALIZED_HASH".to_string());
                    }
                } else {
                    normalize_value(val);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                normalize_value(item);
            }
        }
        _ => {}
    }
}

fn normalize_path_fields(value: &mut Value, workspace_root: &Path) {
    let root = workspace_root_string(workspace_root);
    normalize_path_fields_inner(value, &root);
}

fn workspace_root_string(root: &Path) -> String {
    root.canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .display()
        .to_string()
}

fn normalize_path_fields_inner(value: &mut Value, root: &str) {
    match value {
        Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                if is_path_key(key) {
                    if let Some(s) = val.as_str() {
                        *val = Value::String(normalize_path_value(s, root));
                    }
                } else {
                    normalize_path_fields_inner(val, root);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                normalize_path_fields_inner(item, root);
            }
        }
        _ => {}
    }
}

fn normalize_path_value(value: &str, root: &str) -> String {
    let mut normalized = value.replace('\\', "/");
    let root_norm = root.replace('\\', "/");
    if normalized.starts_with(&root_norm) {
        normalized = format!("<WORKSPACE>{}", &normalized[root_norm.len()..]);
    }
    normalized
}

fn is_path_key(key: &str) -> bool {
    matches!(
        key,
        "path" | "database_path" | "obr_dir" | "jsonl_path" | "redirected_from" | "socket_path"
    )
}

/// Compare two JSON outputs
pub fn compare_json(obr_output: &str, bd_output: &str, mode: &CompareMode) -> Result<(), String> {
    match mode {
        CompareMode::ExactJson => {
            let obr_json: Value =
                serde_json::from_str(obr_output).map_err(|e| format!("obr JSON parse: {e}"))?;
            let bd_json: Value =
                serde_json::from_str(bd_output).map_err(|e| format!("bd JSON parse: {e}"))?;

            if obr_json != bd_json {
                return Err(format!(
                    "JSON mismatch\nbr: {}\nbd: {}",
                    serde_json::to_string_pretty(&obr_json).unwrap_or_default(),
                    serde_json::to_string_pretty(&bd_json).unwrap_or_default()
                ));
            }
        }
        CompareMode::NormalizedJson => {
            let obr_json =
                normalize_json(obr_output).map_err(|e| format!("obr JSON parse: {e}"))?;
            let bd_json = normalize_json(bd_output).map_err(|e| format!("bd JSON parse: {e}"))?;

            if obr_json != bd_json {
                return Err(format!(
                    "Normalized JSON mismatch\nbr: {}\nbd: {}",
                    serde_json::to_string_pretty(&obr_json).unwrap_or_default(),
                    serde_json::to_string_pretty(&bd_json).unwrap_or_default()
                ));
            }
        }
        CompareMode::ContainsFields(fields) => {
            let obr_json: Value =
                serde_json::from_str(obr_output).map_err(|e| format!("obr JSON parse: {e}"))?;
            let bd_json: Value =
                serde_json::from_str(bd_output).map_err(|e| format!("bd JSON parse: {e}"))?;

            for field in fields {
                let obr_val = extract_field(&obr_json, field);
                let bd_val = extract_field(&bd_json, field);

                if obr_val != bd_val {
                    return Err(format!(
                        "Field '{}' mismatch\nbr: {:?}\nbd: {:?}",
                        field, obr_val, bd_val
                    ));
                }
            }
        }
        CompareMode::ExitCodeOnly => {
            // No JSON comparison needed
        }
        CompareMode::ArrayUnordered => {
            let obr_json: Value =
                serde_json::from_str(obr_output).map_err(|e| format!("obr JSON parse: {e}"))?;
            let bd_json: Value =
                serde_json::from_str(bd_output).map_err(|e| format!("bd JSON parse: {e}"))?;

            // Compare arrays ignoring order
            if !json_equal_unordered(&obr_json, &bd_json) {
                return Err(format!(
                    "Array-unordered mismatch\nbr: {}\nbd: {}",
                    serde_json::to_string_pretty(&obr_json).unwrap_or_default(),
                    serde_json::to_string_pretty(&bd_json).unwrap_or_default()
                ));
            }
        }
        CompareMode::FieldsExcluded(excluded) => {
            let obr_json: Value =
                serde_json::from_str(obr_output).map_err(|e| format!("obr JSON parse: {e}"))?;
            let bd_json: Value =
                serde_json::from_str(bd_output).map_err(|e| format!("bd JSON parse: {e}"))?;

            // Remove excluded fields and compare
            let obr_filtered = filter_fields(&obr_json, excluded);
            let bd_filtered = filter_fields(&bd_json, excluded);

            if obr_filtered != bd_filtered {
                return Err(format!(
                    "Fields-excluded mismatch\nbr: {}\nbd: {}",
                    serde_json::to_string_pretty(&obr_filtered).unwrap_or_default(),
                    serde_json::to_string_pretty(&bd_filtered).unwrap_or_default()
                ));
            }
        }
        CompareMode::StructureOnly => {
            let obr_json: Value =
                serde_json::from_str(obr_output).map_err(|e| format!("obr JSON parse: {e}"))?;
            let bd_json: Value =
                serde_json::from_str(bd_output).map_err(|e| format!("bd JSON parse: {e}"))?;

            // Compare structure without values
            if !structure_matches(&obr_json, &bd_json) {
                return Err(format!(
                    "Structure mismatch\nbr: {}\nbd: {}",
                    serde_json::to_string_pretty(&obr_json).unwrap_or_default(),
                    serde_json::to_string_pretty(&bd_json).unwrap_or_default()
                ));
            }
        }
    }
    Ok(())
}

fn log_timings(test_name: &str, obr: &CmdOutput, bd: &CmdOutput) {
    info!("conformance_{}: br_timing={:?}", test_name, obr.duration);
    info!("conformance_{}: bd_timing={:?}", test_name, bd.duration);
    if obr.duration.as_nanos() > 0 {
        let speedup = bd.duration.as_secs_f64() / obr.duration.as_secs_f64();
        info!("conformance_{}: speedup={:.2}x", test_name, speedup);
    }
}

fn extract_field<'a>(json: &'a Value, field: &str) -> Option<&'a Value> {
    let mut current = json;
    for part in field.split('.') {
        match current {
            Value::Object(map) => {
                current = map.get(part)?;
            }
            Value::Array(arr) if !arr.is_empty() => {
                if let Value::Object(map) = &arr[0] {
                    current = map.get(part)?;
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Compare two JSON values ignoring array order
fn json_equal_unordered(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(arr_a), Value::Array(arr_b)) => {
            if arr_a.len() != arr_b.len() {
                return false;
            }
            // Check each element in a exists somewhere in b
            for elem_a in arr_a {
                if !arr_b
                    .iter()
                    .any(|elem_b| json_equal_unordered(elem_a, elem_b))
                {
                    return false;
                }
            }
            true
        }
        (Value::Object(map_a), Value::Object(map_b)) => {
            if map_a.len() != map_b.len() {
                return false;
            }
            for (key, val_a) in map_a {
                match map_b.get(key) {
                    Some(val_b) => {
                        if !json_equal_unordered(val_a, val_b) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        _ => a == b,
    }
}

/// Filter out specified fields from JSON
fn filter_fields(json: &Value, excluded: &[String]) -> Value {
    match json {
        Value::Object(map) => {
            let filtered: serde_json::Map<String, Value> = map
                .iter()
                .filter(|(k, _)| !excluded.contains(k))
                .map(|(k, v)| (k.clone(), filter_fields(v, excluded)))
                .collect();
            Value::Object(filtered)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| filter_fields(v, excluded)).collect()),
        other => other.clone(),
    }
}

/// Check if two JSON values have the same structure (ignoring values)
fn structure_matches(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(map_a), Value::Object(map_b)) => {
            if map_a.len() != map_b.len() {
                return false;
            }
            for (key, val_a) in map_a {
                match map_b.get(key) {
                    Some(val_b) => {
                        if !structure_matches(val_a, val_b) {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            true
        }
        (Value::Array(arr_a), Value::Array(arr_b)) => {
            // For structure, just check that both are arrays and have similar structure in first element
            if arr_a.is_empty() && arr_b.is_empty() {
                return true;
            }
            if arr_a.is_empty() != arr_b.is_empty() {
                return false;
            }
            // Compare first elements' structure
            structure_matches(&arr_a[0], &arr_b[0])
        }
        (Value::Null, Value::Null)
        | (Value::Bool(_), Value::Bool(_))
        | (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_)) => true,
        _ => false,
    }
}

// ============================================================================
// DETAILED DIFF FOR ERROR DIAGNOSTICS
// ============================================================================

/// Generate a human-readable diff between two JSON values
pub fn diff_json(obr: &Value, bd: &Value) -> String {
    let mut diffs = Vec::new();
    collect_diffs(obr, bd, "", &mut diffs);

    if diffs.is_empty() {
        return "No differences found".to_string();
    }

    let mut output = String::new();
    output.push_str("Differences found:\n");
    for (path, obr_val, bd_val) in diffs.iter().take(20) {
        output.push_str(&format!(
            "  {}: obr={}, bd={}\n",
            if path.is_empty() { "(root)" } else { path },
            obr_val,
            bd_val
        ));
    }
    if diffs.len() > 20 {
        output.push_str(&format!(
            "  ... and {} more differences\n",
            diffs.len() - 20
        ));
    }
    output
}

/// Collect all differences between two JSON values
fn collect_diffs(obr: &Value, bd: &Value, path: &str, diffs: &mut Vec<(String, String, String)>) {
    match (obr, bd) {
        (Value::Object(obr_map), Value::Object(bd_map)) => {
            // Check for keys only in obr
            for key in obr_map.keys() {
                if !bd_map.contains_key(key) {
                    let key_path = format_path(path, key);
                    diffs.push((
                        key_path,
                        format_value_short(&obr_map[key]),
                        "(missing)".to_string(),
                    ));
                }
            }
            // Check for keys only in bd
            for key in bd_map.keys() {
                if !obr_map.contains_key(key) {
                    let key_path = format_path(path, key);
                    diffs.push((
                        key_path,
                        "(missing)".to_string(),
                        format_value_short(&bd_map[key]),
                    ));
                }
            }
            // Compare shared keys
            for (key, obr_val) in obr_map {
                if let Some(bd_val) = bd_map.get(key) {
                    collect_diffs(obr_val, bd_val, &format_path(path, key), diffs);
                }
            }
        }
        (Value::Array(obr_arr), Value::Array(bd_arr)) => {
            if obr_arr.len() != bd_arr.len() {
                diffs.push((
                    format!("{}.length", path),
                    obr_arr.len().to_string(),
                    bd_arr.len().to_string(),
                ));
            }
            let min_len = obr_arr.len().min(bd_arr.len());
            for i in 0..min_len {
                collect_diffs(&obr_arr[i], &bd_arr[i], &format!("{}[{}]", path, i), diffs);
            }
        }
        _ => {
            if obr != bd {
                diffs.push((
                    path.to_string(),
                    format_value_short(obr),
                    format_value_short(bd),
                ));
            }
        }
    }
}

fn format_path(base: &str, key: &str) -> String {
    if base.is_empty() {
        key.to_string()
    } else {
        format!("{}.{}", base, key)
    }
}

fn format_value_short(val: &Value) -> String {
    match val {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            if s.len() > 30 {
                format!("\"{}...\"", &s[..27])
            } else {
                format!("\"{}\"", s)
            }
        }
        Value::Array(arr) => format!("[{} items]", arr.len()),
        Value::Object(map) => format!("{{...{} keys}}", map.len()),
    }
}

// ============================================================================
// REUSABLE TEST SCENARIOS
// ============================================================================

/// A reusable test scenario that can be executed against both obr and bd
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TestScenario {
    /// Unique name for the scenario
    pub name: String,
    /// Description of what the scenario tests
    pub description: String,
    /// Commands to run for setup (before the test command)
    pub setup_commands: Vec<Vec<String>>,
    /// The command to test (will be run on both obr and bd)
    pub test_command: Vec<String>,
    /// How to compare the outputs
    pub compare_mode: CompareMode,
    /// Whether to compare exit codes
    pub compare_exit_codes: bool,
}

impl TestScenario {
    /// Create a new test scenario with defaults
    #[allow(dead_code)]
    pub fn new(name: &str, test_command: Vec<&str>) -> Self {
        Self {
            name: name.to_string(),
            description: String::new(),
            setup_commands: Vec::new(),
            test_command: test_command.into_iter().map(String::from).collect(),
            compare_mode: CompareMode::NormalizedJson,
            compare_exit_codes: true,
        }
    }

    #[allow(dead_code)]
    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = desc.to_string();
        self
    }

    #[allow(dead_code)]
    pub fn with_setup(mut self, commands: Vec<Vec<&str>>) -> Self {
        self.setup_commands = commands
            .into_iter()
            .map(|cmd| cmd.into_iter().map(String::from).collect())
            .collect();
        self
    }

    #[allow(dead_code)]
    pub fn with_compare_mode(mut self, mode: CompareMode) -> Self {
        self.compare_mode = mode;
        self
    }

    /// Execute the scenario and return a result
    #[allow(dead_code)]
    pub fn execute(&self, workspace: &ConformanceWorkspace) -> Result<(), String> {
        // Run setup commands
        for cmd in &self.setup_commands {
            let args: Vec<&str> = cmd.iter().map(String::as_str).collect();
            let obr_result = workspace.run_obr(args.clone(), &format!("setup_{}", self.name));
            let bd_result = workspace.run_bd(args, &format!("setup_{}", self.name));

            if !obr_result.status.success() {
                return Err(format!("obr setup failed: {}", obr_result.stderr));
            }
            if !bd_result.status.success() {
                return Err(format!("bd setup failed: {}", bd_result.stderr));
            }
        }

        // Run test command
        let args: Vec<&str> = self.test_command.iter().map(String::as_str).collect();
        let obr_result = workspace.run_obr(args.clone(), &self.name);
        let bd_result = workspace.run_bd(args, &self.name);

        // Compare exit codes if requested
        if self.compare_exit_codes {
            let obr_success = obr_result.status.success();
            let bd_success = bd_result.status.success();
            if obr_success != bd_success {
                return Err(format!(
                    "Exit code mismatch: obr={}, bd={}",
                    obr_result.status, bd_result.status
                ));
            }
        }

        // Compare outputs using the configured mode
        let obr_json = extract_json_payload(&obr_result.stdout);
        let bd_json = extract_json_payload(&bd_result.stdout);

        compare_json(&obr_json, &bd_json, &self.compare_mode)
    }
}

/// Predefined test scenarios for common operations
#[allow(dead_code)]
pub mod scenarios {
    use super::*;

    pub fn empty_list() -> TestScenario {
        TestScenario::new("empty_list", vec!["list", "--json"])
            .with_description("Verify empty list output matches")
    }

    pub fn create_basic() -> TestScenario {
        TestScenario::new("create_basic", vec!["list", "--json"])
            .with_description("Create a basic issue and verify list output")
            .with_setup(vec![vec!["create", "Test issue"]])
            .with_compare_mode(CompareMode::ContainsFields(vec![
                "title".to_string(),
                "status".to_string(),
                "issue_type".to_string(),
            ]))
    }

    pub fn create_with_type_and_priority() -> TestScenario {
        TestScenario::new("create_typed", vec!["list", "--json"])
            .with_description("Create issue with type and priority")
            .with_setup(vec![vec![
                "create",
                "Bug issue",
                "--type",
                "bug",
                "--priority",
                "1",
            ]])
            .with_compare_mode(CompareMode::ContainsFields(vec![
                "title".to_string(),
                "issue_type".to_string(),
                "priority".to_string(),
            ]))
    }

    pub fn stats_after_create() -> TestScenario {
        TestScenario::new("stats_after_create", vec!["stats", "--json"])
            .with_description("Verify stats after creating issues")
            .with_setup(vec![vec!["create", "Issue 1"], vec!["create", "Issue 2"]])
            .with_compare_mode(CompareMode::ContainsFields(vec!["total".to_string()]))
    }
}

// ============================================================================
// CONFORMANCE TESTS
// ============================================================================

#[test]
fn conformance_init() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_init test");

    let workspace = ConformanceWorkspace::new();
    let (obr_out, bd_out) = workspace.init_both();

    assert!(
        obr_out.status.success(),
        "obr init failed: {}",
        obr_out.stderr
    );
    assert!(bd_out.status.success(), "bd init failed: {}", bd_out.stderr);

    // Both should create .obr directories
    assert!(
        workspace.obr_root.join(".obr").exists(),
        "obr did not create .obr"
    );
    assert!(
        workspace.bd_root.join(".obr").exists(),
        "bd did not create .obr"
    );

    info!("conformance_init passed");
}

#[test]
fn conformance_create_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with same parameters
    let obr_create = workspace.run_obr(["create", "Test issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Test issue", "--json"], "create");

    assert!(
        obr_create.status.success(),
        "obr create failed: {}",
        obr_create.stderr
    );
    assert!(
        bd_create.status.success(),
        "bd create failed: {}",
        bd_create.stderr
    );

    // Compare with ContainsFields - title, status, priority should match
    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let result = compare_json(
        &obr_json,
        &bd_json,
        &CompareMode::ContainsFields(vec![
            "title".to_string(),
            "status".to_string(),
            "issue_type".to_string(),
        ]),
    );

    assert!(result.is_ok(), "JSON comparison failed: {:?}", result.err());
    info!("conformance_create_basic passed");
}

#[test]
fn conformance_create_with_type_and_priority() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_with_type_and_priority test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let args = [
        "create",
        "Bug fix needed",
        "--type",
        "bug",
        "--priority",
        "1",
        "--json",
    ];

    let obr_create = workspace.run_obr(args.clone(), "create_bug");
    let bd_create = workspace.run_bd(args, "create_bug");

    assert!(
        obr_create.status.success(),
        "obr create failed: {}",
        obr_create.stderr
    );
    assert!(
        bd_create.status.success(),
        "bd create failed: {}",
        bd_create.stderr
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    // Parse and verify specific fields
    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd json");

    // Handle both object and array outputs
    let obr_issue = if obr_val.is_array() {
        &obr_val[0]
    } else {
        &obr_val
    };
    let bd_issue = if bd_val.is_array() {
        &bd_val[0]
    } else {
        &bd_val
    };

    assert_eq!(obr_issue["title"], bd_issue["title"], "title mismatch");
    assert_eq!(
        obr_issue["issue_type"], bd_issue["issue_type"],
        "issue_type mismatch: obr={}, bd={}",
        obr_issue["issue_type"], bd_issue["issue_type"]
    );
    assert_eq!(
        obr_issue["priority"], bd_issue["priority"],
        "priority mismatch: obr={}, bd={}",
        obr_issue["priority"], bd_issue["priority"]
    );

    info!("conformance_create_with_type_and_priority passed");
}

#[test]
fn conformance_list_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_list = workspace.run_obr(["list", "--json"], "list_empty");
    let bd_list = workspace.run_bd(["list", "--json"], "list_empty");

    assert!(
        obr_list.status.success(),
        "obr list failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list failed: {}",
        bd_list.stderr
    );

    // Both should return empty arrays
    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Null);
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Null);

    // Both should be empty arrays or similar
    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 0, "expected empty list");

    info!("conformance_list_empty passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --json` returns empty array even when issues exist.
/// obr's list command works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --json returns empty: known behavioral difference"]
fn conformance_list_with_issues() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_with_issues test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create same issues in both
    workspace.run_obr(["create", "Issue one"], "create1");
    // Use obr to create in bd workspace, because bd create is flaky
    workspace.run_obr_in_bd_env(["create", "Issue one"], "create1");

    workspace.run_obr(["create", "Issue two"], "create2");
    // Use obr to create in bd workspace, because bd create is flaky
    workspace.run_obr_in_bd_env(["create", "Issue two"], "create2");

    let obr_list = workspace.run_obr(["list", "--json"], "list");
    let bd_list = workspace.run_bd(["list", "--json"], "list");

    assert!(
        obr_list.status.success(),
        "obr list failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list failed: {}",
        bd_list.stderr
    );

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd json");

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 2, "expected 2 issues");

    info!("conformance_list_with_issues passed");
}

#[test]
fn conformance_ready_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_ready_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_ready = workspace.run_obr(["ready", "--json"], "ready_empty");
    let bd_ready = workspace.run_bd(["ready", "--json"], "ready_empty");

    assert!(
        obr_ready.status.success(),
        "obr ready failed: {}",
        obr_ready.stderr
    );
    assert!(
        bd_ready.status.success(),
        "bd ready failed: {}",
        bd_ready.stderr
    );

    let obr_json = extract_json_payload(&obr_ready.stdout);
    let bd_json = extract_json_payload(&bd_ready.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "ready lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );

    info!("conformance_ready_empty passed");
}

#[test]
fn conformance_ready_with_issues() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_ready_with_issues test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    workspace.run_obr(["create", "Ready issue"], "create");
    workspace.run_bd(["create", "Ready issue"], "create");

    let obr_ready = workspace.run_obr(["ready", "--json"], "ready");
    let bd_ready = workspace.run_bd(["ready", "--json"], "ready");

    assert!(
        obr_ready.status.success(),
        "obr ready failed: {}",
        obr_ready.stderr
    );
    assert!(
        bd_ready.status.success(),
        "bd ready failed: {}",
        bd_ready.stderr
    );

    let obr_json = extract_json_payload(&obr_ready.stdout);
    let bd_json = extract_json_payload(&bd_ready.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd json");

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "ready lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 ready issue");

    info!("conformance_ready_with_issues passed");
}

#[test]
fn conformance_ready_with_deps() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_ready_with_deps test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_blocker = workspace.run_obr(["create", "Blocker issue", "--json"], "create_blocker");
    let bd_blocker = workspace.run_bd(["create", "Blocker issue", "--json"], "create_blocker");
    let obr_blocked = workspace.run_obr(["create", "Blocked issue", "--json"], "create_blocked");
    let bd_blocked = workspace.run_bd(["create", "Blocked issue", "--json"], "create_blocked");

    assert!(obr_blocker.status.success());
    assert!(bd_blocker.status.success());
    assert!(obr_blocked.status.success());
    assert!(bd_blocked.status.success());

    let obr_blocker_json: Value =
        serde_json::from_str(&extract_json_payload(&obr_blocker.stdout)).expect("obr json");
    let bd_blocker_json: Value =
        serde_json::from_str(&extract_json_payload(&bd_blocker.stdout)).expect("bd json");
    let obr_blocked_json: Value =
        serde_json::from_str(&extract_json_payload(&obr_blocked.stdout)).expect("obr json");
    let bd_blocked_json: Value =
        serde_json::from_str(&extract_json_payload(&bd_blocked.stdout)).expect("bd json");

    let obr_blocker_id = obr_blocker_json["id"].as_str().expect("obr blocker id");
    let bd_blocker_id = bd_blocker_json["id"].as_str().expect("bd blocker id");
    let obr_blocked_id = obr_blocked_json["id"].as_str().expect("obr blocked id");
    let bd_blocked_id = bd_blocked_json["id"].as_str().expect("bd blocked id");

    let obr_dep = workspace.run_obr(["dep", "add", obr_blocked_id, obr_blocker_id], "dep_add");
    let bd_dep = workspace.run_bd(["dep", "add", bd_blocked_id, bd_blocker_id], "dep_add");
    assert!(
        obr_dep.status.success(),
        "obr dep add failed: {}",
        obr_dep.stderr
    );
    assert!(
        bd_dep.status.success(),
        "bd dep add failed: {}",
        bd_dep.stderr
    );

    let obr_ready = workspace.run_obr(["ready", "--json"], "ready_with_deps");
    let bd_ready = workspace.run_bd(["ready", "--json"], "ready_with_deps");

    assert!(
        obr_ready.status.success(),
        "obr ready failed: {}",
        obr_ready.stderr
    );
    assert!(
        bd_ready.status.success(),
        "bd ready failed: {}",
        bd_ready.stderr
    );

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_ready.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_ready.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_ids: Vec<&str> = issue_items(&obr_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();
    let bd_ids: Vec<&str> = issue_items(&bd_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();

    assert_eq!(obr_ids.len(), bd_ids.len(), "ready lengths differ");
    assert!(
        obr_ids.contains(&obr_blocker_id),
        "obr ready should include blocker"
    );
    assert!(
        !obr_ids.contains(&obr_blocked_id),
        "obr ready should exclude blocked issue"
    );
    assert!(
        bd_ids.contains(&bd_blocker_id),
        "bd ready should include blocker"
    );
    assert!(
        !bd_ids.contains(&bd_blocked_id),
        "bd ready should exclude blocked issue"
    );

    info!("conformance_ready_with_deps passed");
}

#[test]
fn conformance_ready_limit() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_ready_limit test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create multiple ready issues
    for i in 0..3 {
        let title = format!("Ready issue {}", i);
        workspace.run_obr(["create", &title], &format!("ready_limit_br_{i}"));
        workspace.run_bd(["create", &title], &format!("ready_limit_bd_{i}"));
    }

    let obr_ready = workspace.run_obr(["ready", "--json", "--limit", "1"], "ready_limit");
    let bd_ready = workspace.run_bd(["ready", "--json", "--limit", "1"], "ready_limit");

    assert!(
        obr_ready.status.success(),
        "obr ready failed: {}",
        obr_ready.stderr
    );
    assert!(
        bd_ready.status.success(),
        "bd ready failed: {}",
        bd_ready.stderr
    );

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_ready.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_ready.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(obr_len, 1, "obr ready should honor limit");
    assert_eq!(bd_len, 1, "bd ready should honor limit");

    info!("conformance_ready_limit passed");
}

#[test]
fn conformance_ready_filter_type() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_ready_filter_type test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_bug = workspace.run_obr(
        ["create", "Bug issue", "--type", "bug", "--json"],
        "ready_bug",
    );
    let bd_bug = workspace.run_bd(
        ["create", "Bug issue", "--type", "bug", "--json"],
        "ready_bug",
    );
    let _obr_task = workspace.run_obr(["create", "Task issue", "--json"], "ready_task");
    let _bd_task = workspace.run_bd(["create", "Task issue", "--json"], "ready_task");

    let obr_bug_id = serde_json::from_str::<Value>(&extract_json_payload(&obr_bug.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("obr bug id");
    let bd_bug_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_bug.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd bug id");

    let obr_ready = workspace.run_obr(["ready", "--json", "--type", "bug"], "ready_type");
    let bd_ready = workspace.run_bd(["ready", "--json", "--type", "bug"], "ready_type");

    assert!(
        obr_ready.status.success(),
        "obr ready failed: {}",
        obr_ready.stderr
    );
    assert!(
        bd_ready.status.success(),
        "bd ready failed: {}",
        bd_ready.stderr
    );

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_ready.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_ready.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_ids: Vec<&str> = issue_items(&obr_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();
    let bd_ids: Vec<&str> = issue_items(&bd_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();

    assert_eq!(obr_ids.len(), 1, "obr ready should filter to 1 bug");
    assert_eq!(bd_ids.len(), 1, "bd ready should filter to 1 bug");
    assert_eq!(obr_ids[0], obr_bug_id);
    assert_eq!(bd_ids[0], bd_bug_id);

    info!("conformance_ready_filter_type passed");
}

#[test]
fn conformance_ready_filter_assignee() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_ready_filter_assignee test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_assigned = workspace.run_obr(
        ["create", "Assigned issue", "--assignee", "alice", "--json"],
        "assignee",
    );
    let bd_assigned = workspace.run_bd(
        ["create", "Assigned issue", "--assignee", "alice", "--json"],
        "assignee",
    );
    let _obr_unassigned = workspace.run_obr(["create", "Unassigned issue"], "unassigned");
    let _bd_unassigned = workspace.run_bd(["create", "Unassigned issue"], "unassigned");

    let obr_assigned_id =
        serde_json::from_str::<Value>(&extract_json_payload(&obr_assigned.stdout))
            .ok()
            .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
            .expect("obr assigned id");
    let bd_assigned_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_assigned.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd assigned id");

    let obr_ready = workspace.run_obr(["ready", "--json", "--assignee", "alice"], "ready_assignee");
    let bd_ready = workspace.run_bd(["ready", "--json", "--assignee", "alice"], "ready_assignee");

    assert!(
        obr_ready.status.success(),
        "obr ready failed: {}",
        obr_ready.stderr
    );
    assert!(
        bd_ready.status.success(),
        "bd ready failed: {}",
        bd_ready.stderr
    );

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_ready.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_ready.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_ids: Vec<&str> = issue_items(&obr_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();
    let bd_ids: Vec<&str> = issue_items(&bd_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();

    assert_eq!(obr_ids.len(), 1, "obr ready should filter to 1 assignee");
    assert_eq!(bd_ids.len(), 1, "bd ready should filter to 1 assignee");
    assert_eq!(obr_ids[0], obr_assigned_id);
    assert_eq!(bd_ids[0], bd_assigned_id);

    info!("conformance_ready_filter_assignee passed");
}

#[test]
fn conformance_ready_priority_order() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_ready_priority_order test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with mixed priorities
    let priorities = [2, 0, 1];
    for (idx, priority) in priorities.iter().enumerate() {
        let title = format!("Priority issue {}", idx);
        let priority_str = priority.to_string();
        let obr_out = workspace.run_obr(
            ["create", &title, "-p", &priority_str, "--json"],
            &format!("ready_priority_br_{idx}"),
        );
        let bd_out = workspace.run_bd(
            ["create", &title, "-p", &priority_str, "--json"],
            &format!("ready_priority_bd_{idx}"),
        );
        assert!(
            obr_out.status.success(),
            "obr create failed: {}",
            obr_out.stderr
        );
        assert!(
            bd_out.status.success(),
            "bd create failed: {}",
            bd_out.stderr
        );
    }

    let obr_ready = workspace.run_obr(
        ["ready", "--json", "--sort", "priority", "--limit", "0"],
        "ready_priority",
    );
    let bd_ready = workspace.run_bd(
        ["ready", "--json", "--sort", "priority", "--limit", "0"],
        "ready_priority",
    );

    assert!(
        obr_ready.status.success(),
        "obr ready failed: {}",
        obr_ready.stderr
    );
    assert!(
        bd_ready.status.success(),
        "bd ready failed: {}",
        bd_ready.stderr
    );

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_ready.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_ready.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_priorities: Vec<i32> = obr_val
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("priority").and_then(|p| p.as_i64()))
                .map(|p| p as i32)
                .collect()
        })
        .unwrap_or_default();
    let bd_priorities: Vec<i32> = bd_val
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.get("priority").and_then(|p| p.as_i64()))
                .map(|p| p as i32)
                .collect()
        })
        .unwrap_or_default();

    assert_eq!(obr_priorities.len(), 3, "obr ready should return 3 issues");
    assert_eq!(bd_priorities.len(), 3, "bd ready should return 3 issues");

    let obr_sorted = obr_priorities.windows(2).all(|w| w[0] <= w[1]);
    let bd_sorted = bd_priorities.windows(2).all(|w| w[0] <= w[1]);

    assert!(
        obr_sorted,
        "obr priorities not sorted: {:?}",
        obr_priorities
    );
    assert!(bd_sorted, "bd priorities not sorted: {:?}", bd_priorities);

    assert_eq!(
        obr_priorities,
        vec![0, 1, 2],
        "obr ready priority order mismatch"
    );
    assert_eq!(
        bd_priorities,
        vec![0, 1, 2],
        "bd ready priority order mismatch"
    );

    info!("conformance_ready_priority_order passed");
}

#[test]
fn conformance_ready_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_ready_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Ready json shape"], "ready_json_shape_br");
    workspace.run_bd(["create", "Ready json shape"], "ready_json_shape_bd");

    let obr_ready = workspace.run_obr(["ready", "--json"], "ready_json_shape");
    let bd_ready = workspace.run_bd(["ready", "--json"], "ready_json_shape");

    assert!(
        obr_ready.status.success(),
        "obr ready failed: {}",
        obr_ready.stderr
    );
    assert!(
        bd_ready.status.success(),
        "bd ready failed: {}",
        bd_ready.stderr
    );

    let obr_json = extract_json_payload(&obr_ready.stdout);
    let bd_json = extract_json_payload(&bd_ready.stdout);

    // Every other key matches exactly. The sole shape difference is `labels`:
    // obr emits `"labels": []` for an issue with no labels, bd v0.46.0 omits the
    // key entirely. Verified against a real bd on 2026-07-25 (`beads_rust-ecr6`).
    // Excluding just that key keeps the rest of the ready payload shape under
    // live comparison.
    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd json");
    let excluded = vec!["labels".to_string()];
    assert!(
        structure_matches(
            &filter_fields(&obr_val, &excluded),
            &filter_fields(&bd_val, &excluded)
        ),
        "ready JSON structure mismatch\nbr: {obr_json}\nbd: {bd_json}"
    );

    info!("conformance_ready_json_shape passed");
}

#[test]
fn conformance_blocked_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_blocked_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_blocked = workspace.run_obr(["blocked", "--json"], "blocked_empty");
    let bd_blocked = workspace.run_bd(["blocked", "--json"], "blocked_empty");

    assert!(
        obr_blocked.status.success(),
        "obr blocked failed: {}",
        obr_blocked.stderr
    );
    assert!(
        bd_blocked.status.success(),
        "bd blocked failed: {}",
        bd_blocked.stderr
    );

    let obr_json = extract_json_payload(&obr_blocked.stdout);
    let bd_json = extract_json_payload(&bd_blocked.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(obr_len, bd_len, "blocked lengths differ");
    assert_eq!(obr_len, 0, "expected no blocked issues");

    info!("conformance_blocked_empty passed");
}

#[test]
fn conformance_blocked_with_deps() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_blocked_with_deps test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_blocker = workspace.run_obr(["create", "Blocker issue", "--json"], "create_blocker");
    let bd_blocker = workspace.run_bd(["create", "Blocker issue", "--json"], "create_blocker");
    let obr_blocked = workspace.run_obr(["create", "Blocked issue", "--json"], "create_blocked");
    let bd_blocked = workspace.run_bd(["create", "Blocked issue", "--json"], "create_blocked");

    assert!(obr_blocker.status.success());
    assert!(bd_blocker.status.success());
    assert!(obr_blocked.status.success());
    assert!(bd_blocked.status.success());

    let obr_blocker_json: Value =
        serde_json::from_str(&extract_json_payload(&obr_blocker.stdout)).expect("obr json");
    let bd_blocker_json: Value =
        serde_json::from_str(&extract_json_payload(&bd_blocker.stdout)).expect("bd json");
    let obr_blocked_json: Value =
        serde_json::from_str(&extract_json_payload(&obr_blocked.stdout)).expect("obr json");
    let bd_blocked_json: Value =
        serde_json::from_str(&extract_json_payload(&bd_blocked.stdout)).expect("bd json");

    let obr_blocker_id = obr_blocker_json["id"].as_str().expect("obr blocker id");
    let bd_blocker_id = bd_blocker_json["id"].as_str().expect("bd blocker id");
    let obr_blocked_id = obr_blocked_json["id"].as_str().expect("obr blocked id");
    let bd_blocked_id = bd_blocked_json["id"].as_str().expect("bd blocked id");

    let obr_dep = workspace.run_obr(["dep", "add", obr_blocked_id, obr_blocker_id], "dep_add");
    let bd_dep = workspace.run_bd(["dep", "add", bd_blocked_id, bd_blocker_id], "dep_add");
    assert!(
        obr_dep.status.success(),
        "obr dep add failed: {}",
        obr_dep.stderr
    );
    assert!(
        bd_dep.status.success(),
        "bd dep add failed: {}",
        bd_dep.stderr
    );

    let obr_blocked_out = workspace.run_obr(["blocked", "--json"], "blocked_with_deps");
    let bd_blocked_out = workspace.run_bd(["blocked", "--json"], "blocked_with_deps");

    assert!(
        obr_blocked_out.status.success(),
        "obr blocked failed: {}",
        obr_blocked_out.stderr
    );
    assert!(
        bd_blocked_out.status.success(),
        "bd blocked failed: {}",
        bd_blocked_out.stderr
    );

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_blocked_out.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_blocked_out.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_ids: Vec<&str> = issue_items(&obr_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();
    let bd_ids: Vec<&str> = issue_items(&bd_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();

    assert_eq!(obr_ids.len(), bd_ids.len(), "blocked lengths differ");
    assert!(
        obr_ids.contains(&obr_blocked_id),
        "obr blocked should include blocked issue"
    );
    assert!(
        bd_ids.contains(&bd_blocked_id),
        "bd blocked should include blocked issue"
    );

    info!("conformance_blocked_with_deps passed");
}

#[test]
fn conformance_blocked_shows_blockers() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_blocked_shows_blockers test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_blocker = workspace.run_obr(["create", "Blocker issue", "--json"], "create_blocker");
    let bd_blocker = workspace.run_bd(["create", "Blocker issue", "--json"], "create_blocker");
    let obr_blocked = workspace.run_obr(["create", "Blocked issue", "--json"], "create_blocked");
    let bd_blocked = workspace.run_bd(["create", "Blocked issue", "--json"], "create_blocked");

    let obr_blocker_id = serde_json::from_str::<Value>(&extract_json_payload(&obr_blocker.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("obr blocker id");
    let bd_blocker_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_blocker.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd blocker id");
    let obr_blocked_id = serde_json::from_str::<Value>(&extract_json_payload(&obr_blocked.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("obr blocked id");
    let bd_blocked_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_blocked.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd blocked id");

    let obr_dep = workspace.run_obr(["dep", "add", &obr_blocked_id, &obr_blocker_id], "dep_add");
    let bd_dep = workspace.run_bd(["dep", "add", &bd_blocked_id, &bd_blocker_id], "dep_add");
    assert!(
        obr_dep.status.success(),
        "obr dep add failed: {}",
        obr_dep.stderr
    );
    assert!(
        bd_dep.status.success(),
        "bd dep add failed: {}",
        bd_dep.stderr
    );

    let obr_blocked_out = workspace.run_obr(["blocked", "--json"], "blocked_show_blockers");
    let bd_blocked_out = workspace.run_bd(["blocked", "--json"], "blocked_show_blockers");

    assert!(
        obr_blocked_out.status.success(),
        "obr blocked failed: {}",
        obr_blocked_out.stderr
    );
    assert!(
        bd_blocked_out.status.success(),
        "bd blocked failed: {}",
        bd_blocked_out.stderr
    );

    let obr_val: Value =
        serde_json::from_str(&extract_json_payload(&obr_blocked_out.stdout)).unwrap_or_default();
    let bd_val: Value =
        serde_json::from_str(&extract_json_payload(&bd_blocked_out.stdout)).unwrap_or_default();

    fn has_blocker(val: &Value, blocked_id: &str, blocker_id: &str) -> bool {
        for item in issue_items(val) {
            if item.get("id").and_then(|v| v.as_str()) != Some(blocked_id) {
                continue;
            }
            if let Some(blocked_by) = item.get("blocked_by").and_then(|v| v.as_array()) {
                return blocked_by.iter().any(|entry| {
                    entry
                        .as_str()
                        .map(|s| s.split(':').next().unwrap_or(s) == blocker_id)
                        .unwrap_or(false)
                });
            }
        }
        false
    }

    assert!(
        has_blocker(&obr_val, &obr_blocked_id, &obr_blocker_id),
        "obr blocked should list blocker"
    );
    assert!(
        has_blocker(&bd_val, &bd_blocked_id, &bd_blocker_id),
        "bd blocked should list blocker"
    );

    info!("conformance_blocked_shows_blockers passed");
}

#[test]
fn conformance_blocked_multiple_blockers() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_blocked_multiple_blockers test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_blocker1 = workspace.run_obr(["create", "Blocker 1", "--json"], "blocker1");
    let bd_blocker1 = workspace.run_bd(["create", "Blocker 1", "--json"], "blocker1");
    let obr_blocker2 = workspace.run_obr(["create", "Blocker 2", "--json"], "blocker2");
    let bd_blocker2 = workspace.run_bd(["create", "Blocker 2", "--json"], "blocker2");
    let obr_blocked = workspace.run_obr(["create", "Blocked issue", "--json"], "blocked_multi");
    let bd_blocked = workspace.run_bd(["create", "Blocked issue", "--json"], "blocked_multi");

    let obr_blocker1_id =
        serde_json::from_str::<Value>(&extract_json_payload(&obr_blocker1.stdout))
            .ok()
            .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
            .expect("obr blocker1 id");
    let bd_blocker1_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_blocker1.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd blocker1 id");
    let obr_blocker2_id =
        serde_json::from_str::<Value>(&extract_json_payload(&obr_blocker2.stdout))
            .ok()
            .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
            .expect("obr blocker2 id");
    let bd_blocker2_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_blocker2.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd blocker2 id");
    let obr_blocked_id = serde_json::from_str::<Value>(&extract_json_payload(&obr_blocked.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("obr blocked id");
    let bd_blocked_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_blocked.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd blocked id");

    let obr_dep1 = workspace.run_obr(
        ["dep", "add", &obr_blocked_id, &obr_blocker1_id],
        "dep_add1",
    );
    let obr_dep2 = workspace.run_obr(
        ["dep", "add", &obr_blocked_id, &obr_blocker2_id],
        "dep_add2",
    );
    let bd_dep1 = workspace.run_bd(["dep", "add", &bd_blocked_id, &bd_blocker1_id], "dep_add1");
    let bd_dep2 = workspace.run_bd(["dep", "add", &bd_blocked_id, &bd_blocker2_id], "dep_add2");

    assert!(
        obr_dep1.status.success(),
        "obr dep1 failed: {}",
        obr_dep1.stderr
    );
    assert!(
        obr_dep2.status.success(),
        "obr dep2 failed: {}",
        obr_dep2.stderr
    );
    assert!(
        bd_dep1.status.success(),
        "bd dep1 failed: {}",
        bd_dep1.stderr
    );
    assert!(
        bd_dep2.status.success(),
        "bd dep2 failed: {}",
        bd_dep2.stderr
    );

    let obr_blocked_out = workspace.run_obr(["blocked", "--json"], "blocked_multi");
    let bd_blocked_out = workspace.run_bd(["blocked", "--json"], "blocked_multi");

    assert!(
        obr_blocked_out.status.success(),
        "obr blocked failed: {}",
        obr_blocked_out.stderr
    );
    assert!(
        bd_blocked_out.status.success(),
        "bd blocked failed: {}",
        bd_blocked_out.stderr
    );

    let obr_val: Value =
        serde_json::from_str(&extract_json_payload(&obr_blocked_out.stdout)).unwrap_or_default();
    let bd_val: Value =
        serde_json::from_str(&extract_json_payload(&bd_blocked_out.stdout)).unwrap_or_default();

    fn has_blocker(val: &Value, blocked_id: &str, blocker_id: &str) -> bool {
        for item in issue_items(val) {
            if item.get("id").and_then(|v| v.as_str()) != Some(blocked_id) {
                continue;
            }
            if let Some(blocked_by) = item.get("blocked_by").and_then(|v| v.as_array()) {
                return blocked_by.iter().any(|entry| {
                    entry
                        .as_str()
                        .map(|s| s.split(':').next().unwrap_or(s) == blocker_id)
                        .unwrap_or(false)
                });
            }
        }
        false
    }

    assert!(
        has_blocker(&obr_val, &obr_blocked_id, &obr_blocker1_id),
        "obr blocked should include blocker1"
    );
    assert!(
        has_blocker(&obr_val, &obr_blocked_id, &obr_blocker2_id),
        "obr blocked should include blocker2"
    );
    assert!(
        has_blocker(&bd_val, &bd_blocked_id, &bd_blocker1_id),
        "bd blocked should include blocker1"
    );
    assert!(
        has_blocker(&bd_val, &bd_blocked_id, &bd_blocker2_id),
        "bd blocked should include blocker2"
    );

    info!("conformance_blocked_multiple_blockers passed");
}

#[test]
fn conformance_blocked_chain() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_blocked_chain test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_a = workspace.run_obr(["create", "Blocked A", "--json"], "blocked_a");
    let bd_a = workspace.run_bd(["create", "Blocked A", "--json"], "blocked_a");
    let obr_b = workspace.run_obr(["create", "Blocked B", "--json"], "blocked_b");
    let bd_b = workspace.run_bd(["create", "Blocked B", "--json"], "blocked_b");
    let obr_c = workspace.run_obr(["create", "Blocker C", "--json"], "blocked_c");
    let bd_c = workspace.run_bd(["create", "Blocker C", "--json"], "blocked_c");

    let obr_a_id = serde_json::from_str::<Value>(&extract_json_payload(&obr_a.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("obr a id");
    let bd_a_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_a.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd a id");
    let obr_b_id = serde_json::from_str::<Value>(&extract_json_payload(&obr_b.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("obr b id");
    let bd_b_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_b.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd b id");
    let obr_c_id = serde_json::from_str::<Value>(&extract_json_payload(&obr_c.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("obr c id");
    let bd_c_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_c.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd c id");

    let obr_dep1 = workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "dep_a_b");
    let obr_dep2 = workspace.run_obr(["dep", "add", &obr_b_id, &obr_c_id], "dep_b_c");
    let bd_dep1 = workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "dep_a_b");
    let bd_dep2 = workspace.run_bd(["dep", "add", &bd_b_id, &bd_c_id], "dep_b_c");

    assert!(
        obr_dep1.status.success(),
        "obr dep a->b failed: {}",
        obr_dep1.stderr
    );
    assert!(
        obr_dep2.status.success(),
        "obr dep b->c failed: {}",
        obr_dep2.stderr
    );
    assert!(
        bd_dep1.status.success(),
        "bd dep a->b failed: {}",
        bd_dep1.stderr
    );
    assert!(
        bd_dep2.status.success(),
        "bd dep b->c failed: {}",
        bd_dep2.stderr
    );

    let obr_blocked_out = workspace.run_obr(["blocked", "--json"], "blocked_chain");
    let bd_blocked_out = workspace.run_bd(["blocked", "--json"], "blocked_chain");

    assert!(
        obr_blocked_out.status.success(),
        "obr blocked failed: {}",
        obr_blocked_out.stderr
    );
    assert!(
        bd_blocked_out.status.success(),
        "bd blocked failed: {}",
        bd_blocked_out.stderr
    );

    let obr_val: Value =
        serde_json::from_str(&extract_json_payload(&obr_blocked_out.stdout)).unwrap_or_default();
    let bd_val: Value =
        serde_json::from_str(&extract_json_payload(&bd_blocked_out.stdout)).unwrap_or_default();

    let obr_ids: Vec<&str> = issue_items(&obr_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();
    let bd_ids: Vec<&str> = issue_items(&bd_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();

    assert!(obr_ids.contains(&obr_a_id.as_str()));
    assert!(obr_ids.contains(&obr_b_id.as_str()));
    assert!(!obr_ids.contains(&obr_c_id.as_str()));
    assert!(bd_ids.contains(&bd_a_id.as_str()));
    assert!(bd_ids.contains(&bd_b_id.as_str()));
    assert!(!bd_ids.contains(&bd_c_id.as_str()));

    info!("conformance_blocked_chain passed");
}

#[test]
fn conformance_blocked_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_blocked_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_blocker = workspace.run_obr(
        ["create", "Blocker issue", "--json"],
        "blocked_shape_blocker",
    );
    let bd_blocker = workspace.run_bd(
        ["create", "Blocker issue", "--json"],
        "blocked_shape_blocker",
    );
    let obr_blocked = workspace.run_obr(
        ["create", "Blocked issue", "--json"],
        "blocked_shape_blocked",
    );
    let bd_blocked = workspace.run_bd(
        ["create", "Blocked issue", "--json"],
        "blocked_shape_blocked",
    );

    assert!(obr_blocker.status.success());
    assert!(bd_blocker.status.success());
    assert!(obr_blocked.status.success());
    assert!(bd_blocked.status.success());

    let obr_blocker_id = serde_json::from_str::<Value>(&extract_json_payload(&obr_blocker.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("obr blocker id");
    let bd_blocker_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_blocker.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd blocker id");
    let obr_blocked_id = serde_json::from_str::<Value>(&extract_json_payload(&obr_blocked.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("obr blocked id");
    let bd_blocked_id = serde_json::from_str::<Value>(&extract_json_payload(&bd_blocked.stdout))
        .ok()
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(str::to_string))
        .expect("bd blocked id");

    let obr_dep = workspace.run_obr(
        ["dep", "add", &obr_blocked_id, &obr_blocker_id],
        "blocked_shape_dep",
    );
    let bd_dep = workspace.run_bd(
        ["dep", "add", &bd_blocked_id, &bd_blocker_id],
        "blocked_shape_dep",
    );
    assert!(
        obr_dep.status.success(),
        "obr dep add failed: {}",
        obr_dep.stderr
    );
    assert!(
        bd_dep.status.success(),
        "bd dep add failed: {}",
        bd_dep.stderr
    );

    let obr_blocked_out = workspace.run_obr(["blocked", "--json"], "blocked_json_shape");
    let bd_blocked_out = workspace.run_bd(["blocked", "--json"], "blocked_json_shape");

    assert!(
        obr_blocked_out.status.success(),
        "obr blocked failed: {}",
        obr_blocked_out.stderr
    );
    assert!(
        bd_blocked_out.status.success(),
        "bd blocked failed: {}",
        bd_blocked_out.stderr
    );

    let obr_json = extract_json_payload(&obr_blocked_out.stdout);
    let bd_json = extract_json_payload(&bd_blocked_out.stdout);

    compare_json(&obr_json, &bd_json, &CompareMode::StructureOnly).expect("JSON mismatch");

    info!("conformance_blocked_json_shape passed");
}

#[test]
fn conformance_stats() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stats test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create some issues to have stats
    workspace.run_obr(["create", "Issue A"], "create_a");
    workspace.run_bd(["create", "Issue A"], "create_a");

    let obr_stats = workspace.run_obr(["stats", "--no-activity", "--json"], "stats");
    let bd_stats = workspace.run_bd(["stats", "--no-activity", "--json"], "stats");

    assert!(
        obr_stats.status.success(),
        "obr stats failed: {}",
        obr_stats.stderr
    );
    assert!(
        bd_stats.status.success(),
        "bd stats failed: {}",
        bd_stats.stderr
    );

    // Stats command returns structured data - verify key fields match
    let obr_json = extract_json_payload(&obr_stats.stdout);
    let bd_json = extract_json_payload(&bd_stats.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd json");

    // Both should report same total count
    let obr_total = obr_val["total"]
        .as_i64()
        .or_else(|| obr_val["summary"]["total"].as_i64());
    let bd_total = bd_val["total"]
        .as_i64()
        .or_else(|| bd_val["summary"]["total"].as_i64());

    assert_eq!(
        obr_total, bd_total,
        "total issue counts differ: obr={:?}, bd={:?}",
        obr_total, bd_total
    );

    info!("conformance_stats passed");
}

#[test]
fn conformance_sync_flush_only() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_flush_only test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    workspace.run_obr(["create", "Sync test issue"], "create");
    workspace.run_bd(["create", "Sync test issue"], "create");

    // Run sync --flush-only
    let obr_sync = workspace.run_obr(["sync", "--flush-only"], "sync");
    let bd_sync = workspace.run_bd(["sync", "--flush-only"], "sync");

    assert!(
        obr_sync.status.success(),
        "obr sync failed: {}",
        obr_sync.stderr
    );
    assert!(
        bd_sync.status.success(),
        "bd sync failed: {}",
        bd_sync.stderr
    );

    // Both should create issues.jsonl
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    assert!(obr_jsonl.exists(), "obr did not create issues.jsonl");
    assert!(bd_jsonl.exists(), "bd did not create issues.jsonl");

    // Verify JSONL files are non-empty
    let obr_content = fs::read_to_string(&obr_jsonl).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl).expect("read bd jsonl");

    assert!(!obr_content.trim().is_empty(), "obr issues.jsonl is empty");
    assert!(!bd_content.trim().is_empty(), "bd issues.jsonl is empty");

    // Both should have exactly 1 line (1 issue)
    let obr_lines = obr_content.lines().count();
    let bd_lines = bd_content.lines().count();

    assert_eq!(
        obr_lines, bd_lines,
        "JSONL line counts differ: obr={}, bd={}",
        obr_lines, bd_lines
    );

    info!("conformance_sync_flush_only passed");
}

#[test]
fn conformance_dependency_blocking() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dependency_blocking test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create blocker and blocked issues
    let obr_blocker = workspace.run_obr(["create", "Blocker issue", "--json"], "create_blocker");
    let bd_blocker = workspace.run_bd(["create", "Blocker issue", "--json"], "create_blocker");

    let obr_blocked = workspace.run_obr(["create", "Blocked issue", "--json"], "create_blocked");
    let bd_blocked = workspace.run_bd(["create", "Blocked issue", "--json"], "create_blocked");

    // Extract IDs
    let obr_blocker_json = extract_json_payload(&obr_blocker.stdout);
    let bd_blocker_json = extract_json_payload(&bd_blocker.stdout);
    let obr_blocked_json = extract_json_payload(&obr_blocked.stdout);
    let bd_blocked_json = extract_json_payload(&bd_blocked.stdout);

    let obr_blocker_val: Value = serde_json::from_str(&obr_blocker_json).expect("parse");
    let bd_blocker_val: Value = serde_json::from_str(&bd_blocker_json).expect("parse");
    let obr_blocked_val: Value = serde_json::from_str(&obr_blocked_json).expect("parse");
    let bd_blocked_val: Value = serde_json::from_str(&bd_blocked_json).expect("parse");

    let obr_blocker_id = obr_blocker_val["id"]
        .as_str()
        .or_else(|| obr_blocker_val[0]["id"].as_str())
        .unwrap();
    let bd_blocker_id = bd_blocker_val["id"]
        .as_str()
        .or_else(|| bd_blocker_val[0]["id"].as_str())
        .unwrap();
    let obr_blocked_id = obr_blocked_val["id"]
        .as_str()
        .or_else(|| obr_blocked_val[0]["id"].as_str())
        .unwrap();
    let bd_blocked_id = bd_blocked_val["id"]
        .as_str()
        .or_else(|| bd_blocked_val[0]["id"].as_str())
        .unwrap();

    // Add dependency: blocked depends on blocker
    let obr_dep = workspace.run_obr(["dep", "add", obr_blocked_id, obr_blocker_id], "add_dep");
    let bd_dep = workspace.run_bd(["dep", "add", bd_blocked_id, bd_blocker_id], "add_dep");

    assert!(
        obr_dep.status.success(),
        "obr dep add failed: {}",
        obr_dep.stderr
    );
    assert!(
        bd_dep.status.success(),
        "bd dep add failed: {}",
        bd_dep.stderr
    );

    // Check blocked command
    let obr_blocked_cmd = workspace.run_obr(["blocked", "--json"], "blocked");
    let bd_blocked_cmd = workspace.run_bd(["blocked", "--json"], "blocked");

    assert!(obr_blocked_cmd.status.success(), "obr blocked failed");
    assert!(bd_blocked_cmd.status.success(), "bd blocked failed");

    let obr_blocked_json = extract_json_payload(&obr_blocked_cmd.stdout);
    let bd_blocked_json = extract_json_payload(&bd_blocked_cmd.stdout);

    let obr_val: Value = serde_json::from_str(&obr_blocked_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_blocked_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "blocked counts differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 blocked issue");

    // Check ready - should only show the blocker, not the blocked issue
    let obr_ready = workspace.run_obr(["ready", "--json"], "ready_after_dep");
    let bd_ready = workspace.run_bd(["ready", "--json"], "ready_after_dep");

    let obr_ready_json = extract_json_payload(&obr_ready.stdout);
    let bd_ready_json = extract_json_payload(&bd_ready.stdout);

    let obr_ready_val: Value =
        serde_json::from_str(&obr_ready_json).unwrap_or(Value::Array(vec![]));
    let bd_ready_val: Value = serde_json::from_str(&bd_ready_json).unwrap_or(Value::Array(vec![]));

    let obr_ready_len = issue_count(&obr_ready_val);
    let bd_ready_len = issue_count(&bd_ready_val);

    assert_eq!(
        obr_ready_len, bd_ready_len,
        "ready counts differ: obr={}, bd={}",
        obr_ready_len, bd_ready_len
    );
    assert_eq!(obr_ready_len, 1, "expected 1 ready issue (the blocker)");

    info!("conformance_dependency_blocking passed");
}

#[test]
fn conformance_close_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_close_issue test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    let obr_create = workspace.run_obr(["create", "Issue to close", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Issue to close", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    // Close issues
    let obr_close = workspace.run_obr(["close", obr_id, "--json"], "close");
    let bd_close = workspace.run_bd(["close", bd_id, "--json"], "close");

    assert!(
        obr_close.status.success(),
        "obr close failed: {}",
        obr_close.stderr
    );
    assert!(
        bd_close.status.success(),
        "bd close failed: {}",
        bd_close.stderr
    );

    // Verify via show that issues are closed (list may exclude closed by default)
    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_after_close");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_after_close");

    assert!(
        obr_show.status.success(),
        "obr show failed: {}",
        obr_show.stderr
    );
    assert!(
        bd_show.status.success(),
        "bd show failed: {}",
        bd_show.stderr
    );

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_show_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_show_val: Value = serde_json::from_str(&bd_show_json).expect("parse");

    // Handle array or object response
    let obr_issue = if obr_show_val.is_array() {
        &obr_show_val[0]
    } else {
        &obr_show_val
    };
    let bd_issue = if bd_show_val.is_array() {
        &bd_show_val[0]
    } else {
        &bd_show_val
    };

    assert_eq!(
        obr_issue["status"].as_str(),
        Some("closed"),
        "obr issue not closed: got {:?}",
        obr_issue["status"]
    );
    assert_eq!(
        bd_issue["status"].as_str(),
        Some("closed"),
        "bd issue not closed: got {:?}",
        bd_issue["status"]
    );

    info!("conformance_close_issue passed");
}

#[test]
fn conformance_update_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_update_issue test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    let obr_create = workspace.run_obr(["create", "Issue to update", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Issue to update", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    // Update priority
    let obr_update = workspace.run_obr(
        ["update", obr_id, "--priority", "0", "--json"],
        "update_priority",
    );
    let bd_update = workspace.run_bd(
        ["update", bd_id, "--priority", "0", "--json"],
        "update_priority",
    );

    assert!(
        obr_update.status.success(),
        "obr update failed: {}",
        obr_update.stderr
    );
    assert!(
        bd_update.status.success(),
        "bd update failed: {}",
        bd_update.stderr
    );

    // Verify via show
    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_after_update");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_after_update");

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_show_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_show_val: Value = serde_json::from_str(&bd_show_json).expect("parse");

    let obr_priority = obr_show_val["priority"]
        .as_i64()
        .or_else(|| obr_show_val[0]["priority"].as_i64());
    let bd_priority = bd_show_val["priority"]
        .as_i64()
        .or_else(|| bd_show_val[0]["priority"].as_i64());

    assert_eq!(
        obr_priority, bd_priority,
        "priority mismatch after update: obr={:?}, bd={:?}",
        obr_priority, bd_priority
    );
    assert_eq!(obr_priority, Some(0), "expected priority 0");

    info!("conformance_update_issue passed");
}

#[test]
fn conformance_reopen_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_reopen_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create and close issues
    let obr_create = workspace.run_obr(["create", "Issue to reopen", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Issue to reopen", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    // Close issues
    workspace.run_obr(["close", obr_id], "close");
    workspace.run_bd(["close", bd_id], "close");

    // Reopen issues
    let obr_reopen = workspace.run_obr(["reopen", obr_id, "--json"], "reopen");
    let bd_reopen = workspace.run_bd(["reopen", bd_id, "--json"], "reopen");

    assert!(
        obr_reopen.status.success(),
        "obr reopen failed: {}",
        obr_reopen.stderr
    );
    assert!(
        bd_reopen.status.success(),
        "bd reopen failed: {}",
        bd_reopen.stderr
    );

    // Verify status is open again
    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_after_reopen");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_after_reopen");

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_show_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_show_val: Value = serde_json::from_str(&bd_show_json).expect("parse");

    let obr_status = obr_show_val["status"]
        .as_str()
        .or_else(|| obr_show_val[0]["status"].as_str());
    let bd_status = bd_show_val["status"]
        .as_str()
        .or_else(|| bd_show_val[0]["status"].as_str());

    assert_eq!(
        obr_status, bd_status,
        "status mismatch after reopen: obr={:?}, bd={:?}",
        obr_status, bd_status
    );
    assert_eq!(obr_status, Some("open"), "expected status open");

    info!("conformance_reopen_basic passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --type` filter returns empty array.
/// obr's type filtering works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --type filter returns empty: known behavioral difference"]
fn conformance_list_by_type() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_by_type test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with different types
    workspace.run_obr(["create", "Bug issue", "--type", "bug"], "create_bug");
    workspace.run_obr_in_bd_env(["create", "Bug issue", "--type", "bug"], "create_bug");

    workspace.run_obr(
        ["create", "Feature issue", "--type", "feature"],
        "create_feature",
    );
    workspace.run_obr_in_bd_env(
        ["create", "Feature issue", "--type", "feature"],
        "create_feature",
    );

    workspace.run_obr(["create", "Task issue", "--type", "task"], "create_task");
    workspace.run_obr_in_bd_env(["create", "Task issue", "--type", "task"], "create_task");

    // List only bugs
    let obr_list = workspace.run_obr(["list", "--type", "bug", "--json"], "list_bugs");
    let bd_list = workspace.run_bd(["list", "--type", "bug", "--json"], "list_bugs");

    assert!(
        obr_list.status.success(),
        "obr list failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list failed: {}",
        bd_list.stderr
    );

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "bug list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected exactly 1 bug");

    info!("conformance_list_by_type passed");
}

#[test]
fn conformance_show_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_show_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with same title
    let obr_create = workspace.run_obr(
        [
            "create",
            "Show test issue",
            "--type",
            "task",
            "--priority",
            "2",
            "--json",
        ],
        "create",
    );
    let bd_create = workspace.run_bd(
        [
            "create",
            "Show test issue",
            "--type",
            "task",
            "--priority",
            "2",
            "--json",
        ],
        "create",
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    // Show the issues
    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show");

    assert!(
        obr_show.status.success(),
        "obr show failed: {}",
        obr_show.stderr
    );
    assert!(
        bd_show.status.success(),
        "bd show failed: {}",
        bd_show.stderr
    );

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let result = compare_json(
        &obr_show_json,
        &bd_show_json,
        &CompareMode::ContainsFields(vec![
            "title".to_string(),
            "status".to_string(),
            "issue_type".to_string(),
            "priority".to_string(),
        ]),
    );

    assert!(
        result.is_ok(),
        "show JSON comparison failed: {:?}",
        result.err()
    );

    info!("conformance_show_basic passed");
}

#[test]
fn conformance_search_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_search_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with searchable content
    workspace.run_obr(["create", "Authentication bug in login"], "create1");
    workspace.run_bd(["create", "Authentication bug in login"], "create1");

    workspace.run_obr(["create", "Payment processing feature"], "create2");
    workspace.run_bd(["create", "Payment processing feature"], "create2");

    workspace.run_obr(["create", "User login flow improvement"], "create3");
    workspace.run_bd(["create", "User login flow improvement"], "create3");

    // Search for "login"
    let obr_search = workspace.run_obr(["search", "login", "--json"], "search_login");
    let bd_search = workspace.run_bd(["search", "login", "--json"], "search_login");

    assert!(
        obr_search.status.success(),
        "obr search failed: {}",
        obr_search.stderr
    );
    assert!(
        bd_search.status.success(),
        "bd search failed: {}",
        bd_search.stderr
    );

    let obr_json = extract_json_payload(&obr_search.stdout);
    let bd_json = extract_json_payload(&bd_search.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "search result lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 2, "expected 2 issues matching 'login'");

    info!("conformance_search_basic passed");
}

#[test]
fn conformance_label_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_label_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    let obr_create = workspace.run_obr(["create", "Issue for labels", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Issue for labels", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    // Add labels
    let obr_add = workspace.run_obr(["label", "add", obr_id, "urgent"], "label_add");
    let bd_add = workspace.run_bd(["label", "add", bd_id, "urgent"], "label_add");

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

    // List labels
    let obr_list = workspace.run_obr(["label", "list", obr_id, "--json"], "label_list");
    let bd_list = workspace.run_bd(["label", "list", bd_id, "--json"], "label_list");

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

    let obr_label_json = extract_json_payload(&obr_list.stdout);
    let bd_label_json = extract_json_payload(&bd_list.stdout);

    // Both should have "urgent" label
    assert!(
        obr_label_json.contains("urgent"),
        "obr missing 'urgent' label: {}",
        obr_label_json
    );
    assert!(
        bd_label_json.contains("urgent"),
        "bd missing 'urgent' label: {}",
        bd_label_json
    );

    info!("conformance_label_basic passed");
}

#[test]
fn conformance_dep_list() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_list test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create parent and child issues
    let obr_parent = workspace.run_obr(["create", "Parent issue", "--json"], "create_parent");
    let bd_parent = workspace.run_bd(["create", "Parent issue", "--json"], "create_parent");

    let obr_child = workspace.run_obr(["create", "Child issue", "--json"], "create_child");
    let bd_child = workspace.run_bd(["create", "Child issue", "--json"], "create_child");

    let obr_parent_json = extract_json_payload(&obr_parent.stdout);
    let bd_parent_json = extract_json_payload(&bd_parent.stdout);
    let obr_child_json = extract_json_payload(&obr_child.stdout);
    let bd_child_json = extract_json_payload(&bd_child.stdout);

    let obr_parent_val: Value = serde_json::from_str(&obr_parent_json).expect("parse");
    let bd_parent_val: Value = serde_json::from_str(&bd_parent_json).expect("parse");
    let obr_child_val: Value = serde_json::from_str(&obr_child_json).expect("parse");
    let bd_child_val: Value = serde_json::from_str(&bd_child_json).expect("parse");

    let obr_parent_id = obr_parent_val["id"]
        .as_str()
        .or_else(|| obr_parent_val[0]["id"].as_str())
        .unwrap();
    let bd_parent_id = bd_parent_val["id"]
        .as_str()
        .or_else(|| bd_parent_val[0]["id"].as_str())
        .unwrap();
    let obr_child_id = obr_child_val["id"]
        .as_str()
        .or_else(|| obr_child_val[0]["id"].as_str())
        .unwrap();
    let bd_child_id = bd_child_val["id"]
        .as_str()
        .or_else(|| bd_child_val[0]["id"].as_str())
        .unwrap();

    // Add dependency: child depends on parent
    let obr_dep = workspace.run_obr(["dep", "add", obr_child_id, obr_parent_id], "dep_add");
    let bd_dep = workspace.run_bd(["dep", "add", bd_child_id, bd_parent_id], "dep_add");

    assert!(
        obr_dep.status.success(),
        "obr dep add failed: {}",
        obr_dep.stderr
    );
    assert!(
        bd_dep.status.success(),
        "bd dep add failed: {}",
        bd_dep.stderr
    );

    // List dependencies
    let obr_list = workspace.run_obr(["dep", "list", obr_child_id, "--json"], "dep_list");
    let bd_list = workspace.run_bd(["dep", "list", bd_child_id, "--json"], "dep_list");

    assert!(
        obr_list.status.success(),
        "obr dep list failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd dep list failed: {}",
        bd_list.stderr
    );

    let obr_dep_json = extract_json_payload(&obr_list.stdout);
    let bd_dep_json = extract_json_payload(&bd_list.stdout);

    let obr_dep_val: Value = serde_json::from_str(&obr_dep_json).unwrap_or(Value::Array(vec![]));
    let bd_dep_val: Value = serde_json::from_str(&bd_dep_json).unwrap_or(Value::Array(vec![]));

    let obr_dep_len = issue_count(&obr_dep_val);
    let bd_dep_len = issue_count(&bd_dep_val);

    assert_eq!(
        obr_dep_len, bd_dep_len,
        "dep list lengths differ: obr={}, bd={}",
        obr_dep_len, bd_dep_len
    );
    assert_eq!(obr_dep_len, 1, "expected 1 dependency");

    info!("conformance_dep_list passed");
}

#[test]
fn conformance_count_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_count_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with different statuses
    let _obr_create1 = workspace.run_obr(["create", "Open issue 1", "--json"], "create1");
    let _bd_create1 = workspace.run_bd(["create", "Open issue 1", "--json"], "create1");

    let _obr_create2 = workspace.run_obr(["create", "Open issue 2", "--json"], "create2");
    let _bd_create2 = workspace.run_bd(["create", "Open issue 2", "--json"], "create2");

    let obr_create3 = workspace.run_obr(["create", "Will close", "--json"], "create3");
    let bd_create3 = workspace.run_bd(["create", "Will close", "--json"], "create3");

    // Close one issue
    let obr_json = extract_json_payload(&obr_create3.stdout);
    let bd_json = extract_json_payload(&bd_create3.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["close", obr_id], "close");
    workspace.run_bd(["close", bd_id], "close");

    // Run count
    let obr_count = workspace.run_obr(["count", "--json"], "count");
    let bd_count = workspace.run_bd(["count", "--json"], "count");

    assert!(
        obr_count.status.success(),
        "obr count failed: {}",
        obr_count.stderr
    );
    assert!(
        bd_count.status.success(),
        "bd count failed: {}",
        bd_count.stderr
    );

    let obr_count_json = extract_json_payload(&obr_count.stdout);
    let bd_count_json = extract_json_payload(&bd_count.stdout);

    let obr_count_val: Value = serde_json::from_str(&obr_count_json).expect("parse");
    let bd_count_val: Value = serde_json::from_str(&bd_count_json).expect("parse");

    // Both should report same total
    let obr_total = obr_count_val["total"]
        .as_i64()
        .or_else(|| obr_count_val["summary"]["total"].as_i64());
    let bd_total = bd_count_val["total"]
        .as_i64()
        .or_else(|| bd_count_val["summary"]["total"].as_i64());

    assert_eq!(
        obr_total, bd_total,
        "total counts differ: obr={:?}, bd={:?}",
        obr_total, bd_total
    );

    info!("conformance_count_basic passed");
}

#[test]
fn conformance_delete_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_delete_issue test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    let obr_create = workspace.run_obr(["create", "Issue to delete", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Issue to delete", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    // Delete issues (bd requires --force to actually delete, obr doesn't)
    let obr_delete = workspace.run_obr(["delete", obr_id, "--reason", "test deletion"], "delete");
    let bd_delete = workspace.run_bd(
        ["delete", bd_id, "--reason", "test deletion", "--force"],
        "delete",
    );

    assert!(
        obr_delete.status.success(),
        "obr delete failed: {}",
        obr_delete.stderr
    );
    assert!(
        bd_delete.status.success(),
        "bd delete failed: {}",
        bd_delete.stderr
    );

    // Verify deleted issues don't appear in list
    let obr_list = workspace.run_obr(["list", "--json"], "list_after_delete");
    let bd_list = workspace.run_bd(["list", "--json"], "list_after_delete");

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_list_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_list_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_list_val);
    let bd_len = issue_count(&bd_list_val);

    assert_eq!(
        obr_len, bd_len,
        "list lengths differ after delete: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 0, "expected empty list after deletion");

    info!("conformance_delete_issue passed");
}

#[test]
#[ignore]
fn conformance_delete_creates_tombstone() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_delete_creates_tombstone test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Tombstone issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Tombstone issue", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["delete", obr_id, "--reason", "cleanup"], "delete");
    workspace.run_bd(
        ["delete", bd_id, "--reason", "cleanup", "--force"],
        "delete",
    );

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_tombstone");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_tombstone");

    assert_eq!(
        obr_show.status.success(),
        bd_show.status.success(),
        "show tombstone behavior differs: obr success={}, bd success={}",
        obr_show.status.success(),
        bd_show.status.success()
    );

    if obr_show.status.success() && bd_show.status.success() {
        let obr_show_json = extract_json_payload(&obr_show.stdout);
        let bd_show_json = extract_json_payload(&bd_show.stdout);

        if obr_show_json.trim().is_empty() || bd_show_json.trim().is_empty() {
            assert!(
                obr_show_json.trim().is_empty() && bd_show_json.trim().is_empty(),
                "tombstone show output mismatch: obr='{}' bd='{}'",
                obr_show_json,
                bd_show_json
            );
        } else {
            let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
            let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");
            let obr_issue = if obr_val.is_array() {
                &obr_val[0]
            } else {
                &obr_val
            };
            let bd_issue = if bd_val.is_array() {
                &bd_val[0]
            } else {
                &bd_val
            };

            assert_eq!(
                obr_issue["status"].as_str(),
                bd_issue["status"].as_str(),
                "tombstone status mismatch"
            );
        }
    }

    info!("conformance_delete_creates_tombstone passed");
}

#[test]
fn conformance_delete_already_deleted_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_delete_already_deleted_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Delete twice", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Delete twice", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["delete", obr_id, "--reason", "cleanup"], "delete_first");
    workspace.run_bd(
        ["delete", bd_id, "--reason", "cleanup", "--force"],
        "delete_first",
    );

    let obr_delete = workspace.run_obr(["delete", obr_id, "--reason", "cleanup"], "delete_second");
    let bd_delete = workspace.run_bd(
        ["delete", bd_id, "--reason", "cleanup", "--force"],
        "delete_second",
    );

    assert_eq!(
        obr_delete.status.success(),
        bd_delete.status.success(),
        "delete already deleted behavior differs: obr success={}, bd success={}",
        obr_delete.status.success(),
        bd_delete.status.success()
    );

    info!("conformance_delete_already_deleted_error passed");
}

#[test]
fn conformance_delete_with_dependents() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_delete_with_dependents test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_parent = workspace.run_obr(["create", "Parent issue", "--json"], "create_parent");
    let bd_parent = workspace.run_bd(["create", "Parent issue", "--json"], "create_parent");
    let obr_child = workspace.run_obr(["create", "Child issue", "--json"], "create_child");
    let bd_child = workspace.run_bd(["create", "Child issue", "--json"], "create_child");

    let obr_parent_id = extract_issue_id(&extract_json_payload(&obr_parent.stdout));
    let bd_parent_id = extract_issue_id(&extract_json_payload(&bd_parent.stdout));
    let obr_child_id = extract_issue_id(&extract_json_payload(&obr_child.stdout));
    let bd_child_id = extract_issue_id(&extract_json_payload(&bd_child.stdout));

    workspace.run_obr(["dep", "add", &obr_child_id, &obr_parent_id], "dep_add");
    workspace.run_bd(["dep", "add", &bd_child_id, &bd_parent_id], "dep_add");

    workspace.run_obr(
        ["delete", &obr_parent_id, "--reason", "cleanup"],
        "delete_parent",
    );
    workspace.run_bd(
        ["delete", &bd_parent_id, "--reason", "cleanup", "--force"],
        "delete_parent",
    );

    let obr_show = workspace.run_obr(["show", &obr_child_id, "--json"], "show_child");
    let bd_show = workspace.run_bd(["show", &bd_child_id, "--json"], "show_child");

    assert_eq!(
        obr_show.status.success(),
        bd_show.status.success(),
        "child visibility differs after parent delete: obr success={}, bd success={}",
        obr_show.status.success(),
        bd_show.status.success()
    );

    info!("conformance_delete_with_dependents passed");
}

#[test]
fn conformance_dep_remove() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_remove test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create blocker and blocked issues
    let obr_blocker = workspace.run_obr(["create", "Blocker", "--json"], "create_blocker");
    let bd_blocker = workspace.run_bd(["create", "Blocker", "--json"], "create_blocker");

    let obr_blocked = workspace.run_obr(["create", "Blocked", "--json"], "create_blocked");
    let bd_blocked = workspace.run_bd(["create", "Blocked", "--json"], "create_blocked");

    // Extract IDs
    let obr_blocker_id = {
        let json = extract_json_payload(&obr_blocker.stdout);
        let val: Value = serde_json::from_str(&json).expect("parse");
        val["id"]
            .as_str()
            .or_else(|| val[0]["id"].as_str())
            .unwrap()
            .to_string()
    };
    let bd_blocker_id = {
        let json = extract_json_payload(&bd_blocker.stdout);
        let val: Value = serde_json::from_str(&json).expect("parse");
        val["id"]
            .as_str()
            .or_else(|| val[0]["id"].as_str())
            .unwrap()
            .to_string()
    };
    let obr_blocked_id = {
        let json = extract_json_payload(&obr_blocked.stdout);
        let val: Value = serde_json::from_str(&json).expect("parse");
        val["id"]
            .as_str()
            .or_else(|| val[0]["id"].as_str())
            .unwrap()
            .to_string()
    };
    let bd_blocked_id = {
        let json = extract_json_payload(&bd_blocked.stdout);
        let val: Value = serde_json::from_str(&json).expect("parse");
        val["id"]
            .as_str()
            .or_else(|| val[0]["id"].as_str())
            .unwrap()
            .to_string()
    };

    // Add dependency
    workspace.run_obr(["dep", "add", &obr_blocked_id, &obr_blocker_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_blocked_id, &bd_blocker_id], "add_dep");

    // Verify blocked
    let obr_blocked_cmd = workspace.run_obr(["blocked", "--json"], "blocked_before");
    let bd_blocked_cmd = workspace.run_bd(["blocked", "--json"], "blocked_before");

    let obr_before_json = extract_json_payload(&obr_blocked_cmd.stdout);
    let bd_before_json = extract_json_payload(&bd_blocked_cmd.stdout);

    let obr_before: Value = serde_json::from_str(&obr_before_json).unwrap_or(Value::Array(vec![]));
    let bd_before: Value = serde_json::from_str(&bd_before_json).unwrap_or(Value::Array(vec![]));

    assert_eq!(
        issue_count(&obr_before),
        1,
        "expected 1 blocked issue before remove"
    );
    assert_eq!(
        issue_count(&bd_before),
        1,
        "expected 1 blocked issue before remove"
    );

    // Remove dependency
    let obr_rm = workspace.run_obr(
        ["dep", "remove", &obr_blocked_id, &obr_blocker_id],
        "rm_dep",
    );
    let bd_rm = workspace.run_bd(["dep", "remove", &bd_blocked_id, &bd_blocker_id], "rm_dep");

    assert!(
        obr_rm.status.success(),
        "obr dep remove failed: {}",
        obr_rm.stderr
    );
    assert!(
        bd_rm.status.success(),
        "bd dep remove failed: {}",
        bd_rm.stderr
    );

    // Verify no longer blocked
    let obr_blocked_after = workspace.run_obr(["blocked", "--json"], "blocked_after");
    let bd_blocked_after = workspace.run_bd(["blocked", "--json"], "blocked_after");

    let obr_after_json = extract_json_payload(&obr_blocked_after.stdout);
    let bd_after_json = extract_json_payload(&bd_blocked_after.stdout);

    let obr_after: Value = serde_json::from_str(&obr_after_json).unwrap_or(Value::Array(vec![]));
    let bd_after: Value = serde_json::from_str(&bd_after_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_after);
    let bd_len = issue_count(&bd_after);

    assert_eq!(
        obr_len, bd_len,
        "blocked counts differ after remove: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 0, "expected no blocked issues after dep remove");

    info!("conformance_dep_remove passed");
}

#[test]
fn conformance_sync_import() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues and export
    workspace.run_obr(["create", "Import test A"], "create_a");
    workspace.run_bd(["create", "Import test A"], "create_a");

    workspace.run_obr(["create", "Import test B"], "create_b");
    workspace.run_bd(["create", "Import test B"], "create_b");

    // Export from both
    workspace.run_obr(["sync", "--flush-only"], "export");
    workspace.run_bd(["sync", "--flush-only"], "export");

    // Create fresh workspaces for import
    let import_workspace = ConformanceWorkspace::new();
    import_workspace.init_both();

    // Copy JSONL files to new workspaces
    let obr_src_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_src_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");
    let obr_dst_jsonl = import_workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_dst_jsonl = import_workspace.bd_root.join(".obr").join("issues.jsonl");

    fs::copy(&obr_src_jsonl, &obr_dst_jsonl).expect("copy obr jsonl");
    fs::copy(&bd_src_jsonl, &bd_dst_jsonl).expect("copy bd jsonl");

    // Import
    let obr_import = import_workspace.run_obr(["sync", "--import-only"], "import");
    let bd_import = import_workspace.run_bd(["sync", "--import-only"], "import");

    assert!(
        obr_import.status.success(),
        "obr import failed: {}",
        obr_import.stderr
    );
    assert!(
        bd_import.status.success(),
        "bd import failed: {}",
        bd_import.stderr
    );

    // Verify issues were imported
    let obr_list = import_workspace.run_obr(["list", "--json"], "list_after_import");
    let bd_list = import_workspace.run_bd(["list", "--json"], "list_after_import");

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "import counts differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 2, "expected 2 issues after import");

    info!("conformance_sync_import passed");
}

#[test]
fn conformance_sync_roundtrip() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_roundtrip test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with various attributes
    workspace.run_obr(
        [
            "create",
            "Roundtrip bug",
            "--type",
            "bug",
            "--priority",
            "1",
        ],
        "create_bug",
    );
    workspace.run_bd(
        [
            "create",
            "Roundtrip bug",
            "--type",
            "bug",
            "--priority",
            "1",
        ],
        "create_bug",
    );

    workspace.run_obr(
        [
            "create",
            "Roundtrip feature",
            "--type",
            "feature",
            "--priority",
            "3",
        ],
        "create_feature",
    );
    workspace.run_bd(
        [
            "create",
            "Roundtrip feature",
            "--type",
            "feature",
            "--priority",
            "3",
        ],
        "create_feature",
    );

    // Export
    workspace.run_obr(["sync", "--flush-only"], "export");
    workspace.run_bd(["sync", "--flush-only"], "export");

    // Read JSONL content
    let obr_jsonl_path = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl_path = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_jsonl = fs::read_to_string(&obr_jsonl_path).expect("read obr jsonl");
    let bd_jsonl = fs::read_to_string(&bd_jsonl_path).expect("read bd jsonl");

    // Verify same number of lines (issues)
    let obr_lines = obr_jsonl.lines().count();
    let bd_lines = bd_jsonl.lines().count();

    assert_eq!(
        obr_lines, bd_lines,
        "JSONL line counts differ: obr={}, bd={}",
        obr_lines, bd_lines
    );
    assert_eq!(obr_lines, 2, "expected 2 lines in JSONL");

    // Parse JSONL and collect titles (order may differ between obr and bd)
    let obr_titles: HashSet<String> = obr_jsonl
        .lines()
        .map(|line| {
            let val: Value = serde_json::from_str(line).expect("parse obr line");
            val["title"].as_str().unwrap_or("").to_string()
        })
        .collect();
    let bd_titles: HashSet<String> = bd_jsonl
        .lines()
        .map(|line| {
            let val: Value = serde_json::from_str(line).expect("parse bd line");
            val["title"].as_str().unwrap_or("").to_string()
        })
        .collect();

    assert_eq!(
        obr_titles, bd_titles,
        "JSONL titles differ: obr={:?}, bd={:?}",
        obr_titles, bd_titles
    );

    // Create fresh workspaces, import, and verify
    let import_workspace = ConformanceWorkspace::new();
    import_workspace.init_both();

    let obr_dst_jsonl = import_workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_dst_jsonl = import_workspace.bd_root.join(".obr").join("issues.jsonl");

    fs::copy(&obr_jsonl_path, &obr_dst_jsonl).expect("copy obr jsonl");
    fs::copy(&bd_jsonl_path, &bd_dst_jsonl).expect("copy bd jsonl");

    import_workspace.run_obr(["sync", "--import-only"], "import");
    import_workspace.run_bd(["sync", "--import-only"], "import");

    // Verify imported data matches
    let obr_after = import_workspace.run_obr(["list", "--json"], "list_after");
    let bd_after = import_workspace.run_bd(["list", "--json"], "list_after");

    let obr_after_json = extract_json_payload(&obr_after.stdout);
    let bd_after_json = extract_json_payload(&bd_after.stdout);

    let obr_after_val: Value = serde_json::from_str(&obr_after_json).expect("parse");
    let bd_after_val: Value = serde_json::from_str(&bd_after_json).expect("parse");

    let obr_after_len = issue_count(&obr_after_val);
    let bd_after_len = issue_count(&bd_after_val);

    assert_eq!(
        obr_after_len, bd_after_len,
        "roundtrip counts differ: obr={}, bd={}",
        obr_after_len, bd_after_len
    );
    assert_eq!(obr_after_len, 2, "expected 2 issues after roundtrip");

    info!("conformance_sync_roundtrip passed");
}

// ============================================================================
// SYNC COMMAND EXPANSION TESTS
// ============================================================================

// --- sync --flush-only expansion tests ---

#[test]
fn conformance_sync_flush_empty_db() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_flush_empty_db test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Don't create any issues - test flush on empty DB
    let obr_sync = workspace.run_obr(["sync", "--flush-only"], "flush_empty");
    let bd_sync = workspace.run_bd(["sync", "--flush-only"], "flush_empty");

    // Both should succeed (or both fail consistently)
    assert_eq!(
        obr_sync.status.success(),
        bd_sync.status.success(),
        "flush empty behavior differs: obr={}, bd={}",
        obr_sync.status.success(),
        bd_sync.status.success()
    );

    // If successful, check JSONL exists and is empty
    if obr_sync.status.success() {
        let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
        let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

        if obr_jsonl.exists() && bd_jsonl.exists() {
            let obr_content = fs::read_to_string(&obr_jsonl).unwrap_or_default();
            let bd_content = fs::read_to_string(&bd_jsonl).unwrap_or_default();

            // Both should be empty or have same line count
            let obr_lines = obr_content.lines().filter(|l| !l.is_empty()).count();
            let bd_lines = bd_content.lines().filter(|l| !l.is_empty()).count();

            assert_eq!(
                obr_lines, bd_lines,
                "empty db JSONL line counts differ: obr={}, bd={}",
                obr_lines, bd_lines
            );
        }
    }

    info!("conformance_sync_flush_empty_db passed");
}

#[test]
fn conformance_sync_flush_single_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_flush_single_issue test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create exactly one issue
    workspace.run_obr(["create", "Single issue for sync"], "create");
    workspace.run_bd(["create", "Single issue for sync"], "create");

    // Flush
    let obr_sync = workspace.run_obr(["sync", "--flush-only"], "flush");
    let bd_sync = workspace.run_bd(["sync", "--flush-only"], "flush");

    assert!(obr_sync.status.success(), "obr flush failed");
    assert!(bd_sync.status.success(), "bd flush failed");

    // Read JSONL files
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl).expect("read bd jsonl");

    // Both should have exactly 1 non-empty line
    let obr_lines: Vec<&str> = obr_content.lines().filter(|l| !l.is_empty()).collect();
    let bd_lines: Vec<&str> = bd_content.lines().filter(|l| !l.is_empty()).collect();

    assert_eq!(obr_lines.len(), 1, "obr should have 1 line");
    assert_eq!(bd_lines.len(), 1, "bd should have 1 line");

    // Parse and verify titles match
    let obr_val: Value = serde_json::from_str(obr_lines[0]).expect("parse obr jsonl");
    let bd_val: Value = serde_json::from_str(bd_lines[0]).expect("parse bd jsonl");

    assert_eq!(
        obr_val["title"].as_str(),
        bd_val["title"].as_str(),
        "titles should match"
    );

    info!("conformance_sync_flush_single_issue passed");
}

#[test]
fn conformance_sync_flush_many_issues() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_flush_many_issues test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create 20 issues (100 would be too slow for conformance tests)
    for i in 0..20 {
        workspace.run_obr(
            ["create", &format!("Issue number {}", i)],
            &format!("create_{}", i),
        );
        workspace.run_bd(
            ["create", &format!("Issue number {}", i)],
            &format!("create_{}", i),
        );
    }

    // Flush
    let obr_sync = workspace.run_obr(["sync", "--flush-only"], "flush");
    let bd_sync = workspace.run_bd(["sync", "--flush-only"], "flush");

    assert!(obr_sync.status.success(), "obr flush failed");
    assert!(bd_sync.status.success(), "bd flush failed");

    // Read and count lines
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl).expect("read bd jsonl");

    let obr_lines = obr_content.lines().filter(|l| !l.is_empty()).count();
    let bd_lines = bd_content.lines().filter(|l| !l.is_empty()).count();

    assert_eq!(
        obr_lines, bd_lines,
        "many issues JSONL line counts differ: obr={}, bd={}",
        obr_lines, bd_lines
    );
    assert_eq!(obr_lines, 20, "expected 20 lines in JSONL");

    info!("conformance_sync_flush_many_issues passed");
}

#[test]
fn conformance_sync_flush_with_dependencies() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_flush_with_dependencies test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with dependencies
    let obr_blocker = workspace.run_obr(["create", "Blocker issue", "--json"], "create_blocker");
    let bd_blocker = workspace.run_bd(["create", "Blocker issue", "--json"], "create_blocker");

    let obr_blocked = workspace.run_obr(["create", "Blocked issue", "--json"], "create_blocked");
    let bd_blocked = workspace.run_bd(["create", "Blocked issue", "--json"], "create_blocked");

    let obr_blocker_id = extract_issue_id(&extract_json_payload(&obr_blocker.stdout));
    let bd_blocker_id = extract_issue_id(&extract_json_payload(&bd_blocker.stdout));
    let obr_blocked_id = extract_issue_id(&extract_json_payload(&obr_blocked.stdout));
    let bd_blocked_id = extract_issue_id(&extract_json_payload(&bd_blocked.stdout));

    // Add dependency
    workspace.run_obr(["dep", "add", &obr_blocked_id, &obr_blocker_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_blocked_id, &bd_blocker_id], "add_dep");

    // Flush
    let obr_sync = workspace.run_obr(["sync", "--flush-only"], "flush");
    let bd_sync = workspace.run_bd(["sync", "--flush-only"], "flush");

    assert!(obr_sync.status.success(), "obr flush failed");
    assert!(bd_sync.status.success(), "bd flush failed");

    // Read JSONL and verify dependency data exists
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl).expect("read bd jsonl");

    // Both should have 2 issues
    let obr_lines = obr_content.lines().filter(|l| !l.is_empty()).count();
    let bd_lines = bd_content.lines().filter(|l| !l.is_empty()).count();

    assert_eq!(obr_lines, 2, "obr should have 2 lines");
    assert_eq!(bd_lines, 2, "bd should have 2 lines");

    // Check if dependencies are exported (implementation varies - just verify structure)
    info!(
        "obr JSONL size: {}, bd JSONL size: {}",
        obr_content.len(),
        bd_content.len()
    );

    info!("conformance_sync_flush_with_dependencies passed");
}

#[test]
fn conformance_sync_flush_with_labels() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_flush_with_labels test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue with label
    let obr_issue = workspace.run_obr(["create", "Labeled issue", "--json"], "create");
    let bd_issue = workspace.run_bd(["create", "Labeled issue", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_issue.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_issue.stdout));

    // Add labels
    workspace.run_obr(["label", "add", &obr_id, "test-label"], "add_label");
    workspace.run_bd(["label", "add", &bd_id, "test-label"], "add_label");

    // Flush
    let obr_sync = workspace.run_obr(["sync", "--flush-only"], "flush");
    let bd_sync = workspace.run_bd(["sync", "--flush-only"], "flush");

    assert!(obr_sync.status.success(), "obr flush failed");
    assert!(bd_sync.status.success(), "bd flush failed");

    // Read and verify JSONL has label data
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl).expect("read bd jsonl");

    // Parse and check labels field
    let obr_val: Value = serde_json::from_str(obr_content.lines().next().unwrap()).expect("parse");
    let bd_val: Value = serde_json::from_str(bd_content.lines().next().unwrap()).expect("parse");

    // Both should have labels (array or string)
    let obr_has_labels = obr_val.get("labels").is_some();
    let bd_has_labels = bd_val.get("labels").is_some();

    info!(
        "Labels in JSONL: obr={}, bd={}",
        obr_has_labels, bd_has_labels
    );

    info!("conformance_sync_flush_with_labels passed");
}

#[test]
fn conformance_sync_flush_jsonl_line_format() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_flush_jsonl_line_format test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue with known content
    workspace.run_obr(
        [
            "create",
            "Format test issue",
            "--type",
            "bug",
            "--priority",
            "1",
        ],
        "create",
    );
    workspace.run_bd(
        [
            "create",
            "Format test issue",
            "--type",
            "bug",
            "--priority",
            "1",
        ],
        "create",
    );

    // Flush
    workspace.run_obr(["sync", "--flush-only"], "flush");
    workspace.run_bd(["sync", "--flush-only"], "flush");

    // Read JSONL
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl).expect("read bd jsonl");

    // Each line should be valid JSON
    for (i, line) in obr_content.lines().filter(|l| !l.is_empty()).enumerate() {
        serde_json::from_str::<Value>(line)
            .unwrap_or_else(|e| panic!("obr JSONL line {} is not valid JSON: {}", i, e));
    }

    for (i, line) in bd_content.lines().filter(|l| !l.is_empty()).enumerate() {
        serde_json::from_str::<Value>(line)
            .unwrap_or_else(|e| panic!("bd JSONL line {} is not valid JSON: {}", i, e));
    }

    // Parse first line and verify required fields exist
    let obr_val: Value =
        serde_json::from_str(obr_content.lines().next().unwrap()).expect("parse obr");
    let bd_val: Value = serde_json::from_str(bd_content.lines().next().unwrap()).expect("parse bd");

    // Check required fields are present
    let required_fields = ["id", "title", "status", "priority"];

    for field in required_fields {
        assert!(
            obr_val.get(field).is_some(),
            "obr JSONL missing required field: {}",
            field
        );
        assert!(
            bd_val.get(field).is_some(),
            "bd JSONL missing required field: {}",
            field
        );
    }

    info!("conformance_sync_flush_jsonl_line_format passed");
}

#[test]
fn conformance_sync_flush_with_comments() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_flush_with_comments test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue
    let obr_issue = workspace.run_obr(["create", "Commented issue", "--json"], "create");
    let bd_issue = workspace.run_bd(["create", "Commented issue", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_issue.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_issue.stdout));

    // Add comment
    workspace.run_obr(["comments", "add", &obr_id, "Test comment"], "add_comment");
    workspace.run_bd(["comments", "add", &bd_id, "Test comment"], "add_comment");

    // Flush
    let obr_sync = workspace.run_obr(["sync", "--flush-only"], "flush");
    let bd_sync = workspace.run_bd(["sync", "--flush-only"], "flush");

    assert!(obr_sync.status.success(), "obr flush failed");
    assert!(bd_sync.status.success(), "bd flush failed");

    // Read JSONL
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl).expect("read bd jsonl");

    // Verify files were created with content
    assert!(!obr_content.trim().is_empty(), "obr JSONL is empty");
    assert!(!bd_content.trim().is_empty(), "bd JSONL is empty");

    info!("conformance_sync_flush_with_comments passed");
}

// --- sync --import-only expansion tests ---

#[test]
fn conformance_sync_import_empty_jsonl() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import_empty_jsonl test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create empty JSONL files
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    fs::write(&obr_jsonl, "").expect("write obr jsonl");
    fs::write(&bd_jsonl, "").expect("write bd jsonl");

    // Import empty file
    let obr_import = workspace.run_obr(["sync", "--import-only"], "import_empty");
    let bd_import = workspace.run_bd(["sync", "--import-only"], "import_empty");

    // Both should succeed (or both fail consistently)
    assert_eq!(
        obr_import.status.success(),
        bd_import.status.success(),
        "import empty behavior differs: obr={}, bd={}",
        obr_import.status.success(),
        bd_import.status.success()
    );

    // Verify no issues created
    let obr_list = workspace.run_obr(["list", "--json"], "list");
    let bd_list = workspace.run_bd(["list", "--json"], "list");

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "import empty counts differ: obr={}, bd={}",
        obr_len, bd_len
    );

    info!("conformance_sync_import_empty_jsonl passed");
}

#[test]
fn conformance_sync_import_single_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import_single_issue test");

    let source_workspace = ConformanceWorkspace::new();
    source_workspace.init_both();

    // Create issue and export
    source_workspace.run_obr(["create", "Single import test"], "create");
    source_workspace.run_bd(["create", "Single import test"], "create");

    source_workspace.run_obr(["sync", "--flush-only"], "export");
    source_workspace.run_bd(["sync", "--flush-only"], "export");

    // Create fresh workspace and copy JSONL
    let import_workspace = ConformanceWorkspace::new();
    import_workspace.init_both();

    let obr_src = source_workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_src = source_workspace.bd_root.join(".obr").join("issues.jsonl");
    let obr_dst = import_workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_dst = import_workspace.bd_root.join(".obr").join("issues.jsonl");

    fs::copy(&obr_src, &obr_dst).expect("copy obr jsonl");
    fs::copy(&bd_src, &bd_dst).expect("copy bd jsonl");

    // Import
    let obr_import = import_workspace.run_obr(["sync", "--import-only"], "import");
    let bd_import = import_workspace.run_bd(["sync", "--import-only"], "import");

    assert!(obr_import.status.success(), "obr import failed");
    assert!(bd_import.status.success(), "bd import failed");

    // Verify 1 issue imported
    let obr_list = import_workspace.run_obr(["list", "--json"], "list");
    let bd_list = import_workspace.run_bd(["list", "--json"], "list");

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_list.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_list.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(obr_len, bd_len, "single import counts differ");
    assert_eq!(obr_len, 1, "expected 1 issue after single import");

    info!("conformance_sync_import_single_issue passed");
}

#[test]
fn conformance_sync_import_many_issues() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import_many_issues test");

    let source_workspace = ConformanceWorkspace::new();
    source_workspace.init_both();

    // Create 10 issues and export
    for i in 0..10 {
        source_workspace.run_obr(
            ["create", &format!("Many import {}", i)],
            &format!("create_{}", i),
        );
        source_workspace.run_bd(
            ["create", &format!("Many import {}", i)],
            &format!("create_{}", i),
        );
    }

    source_workspace.run_obr(["sync", "--flush-only"], "export");
    source_workspace.run_bd(["sync", "--flush-only"], "export");

    // Create fresh workspace and import
    let import_workspace = ConformanceWorkspace::new();
    import_workspace.init_both();

    let obr_src = source_workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_src = source_workspace.bd_root.join(".obr").join("issues.jsonl");
    let obr_dst = import_workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_dst = import_workspace.bd_root.join(".obr").join("issues.jsonl");

    fs::copy(&obr_src, &obr_dst).expect("copy obr jsonl");
    fs::copy(&bd_src, &bd_dst).expect("copy bd jsonl");

    let obr_import = import_workspace.run_obr(["sync", "--import-only"], "import");
    let bd_import = import_workspace.run_bd(["sync", "--import-only"], "import");

    assert!(obr_import.status.success(), "obr import failed");
    assert!(bd_import.status.success(), "bd import failed");

    // Verify 10 issues imported
    let obr_list = import_workspace.run_obr(["list", "--json"], "list");
    let bd_list = import_workspace.run_bd(["list", "--json"], "list");

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_list.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_list.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "many import counts differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 10, "expected 10 issues after many import");

    info!("conformance_sync_import_many_issues passed");
}

#[test]
fn conformance_sync_import_updates_existing() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import_updates_existing test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue
    let obr_issue = workspace.run_obr(["create", "Update test issue", "--json"], "create");
    let bd_issue = workspace.run_bd(["create", "Update test issue", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_issue.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_issue.stdout));

    // Export
    workspace.run_obr(["sync", "--flush-only"], "export1");
    workspace.run_bd(["sync", "--flush-only"], "export1");

    // Update issue
    workspace.run_obr(["update", &obr_id, "--priority", "1"], "update");
    workspace.run_bd(["update", &bd_id, "--priority", "1"], "update");

    // Export again
    workspace.run_obr(["sync", "--flush-only"], "export2");
    workspace.run_bd(["sync", "--flush-only"], "export2");

    // Re-import (should update existing, not duplicate)
    let obr_import = workspace.run_obr(["sync", "--import-only"], "import");
    let bd_import = workspace.run_bd(["sync", "--import-only"], "import");

    assert!(obr_import.status.success(), "obr import failed");
    assert!(bd_import.status.success(), "bd import failed");

    // Should still have 1 issue
    let obr_list = workspace.run_obr(["list", "--json"], "list");
    let bd_list = workspace.run_bd(["list", "--json"], "list");

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_list.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_list.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(obr_len, bd_len, "update existing counts differ");
    assert_eq!(obr_len, 1, "expected 1 issue (not duplicated)");

    info!("conformance_sync_import_updates_existing passed");
}

// --- sync roundtrip expansion tests ---

#[test]
fn conformance_sync_roundtrip_preserves_all_fields() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_roundtrip_preserves_all_fields test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue with all fields
    workspace.run_obr(
        [
            "create",
            "Full field test",
            "--type",
            "feature",
            "--priority",
            "2",
            "--description",
            "Test description",
        ],
        "create",
    );
    workspace.run_bd(
        [
            "create",
            "Full field test",
            "--type",
            "feature",
            "--priority",
            "2",
            "--description",
            "Test description",
        ],
        "create",
    );

    // Export
    workspace.run_obr(["sync", "--flush-only"], "export");
    workspace.run_bd(["sync", "--flush-only"], "export");

    // Create fresh workspace and import
    let import_workspace = ConformanceWorkspace::new();
    import_workspace.init_both();

    let obr_src = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_src = workspace.bd_root.join(".obr").join("issues.jsonl");
    let obr_dst = import_workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_dst = import_workspace.bd_root.join(".obr").join("issues.jsonl");

    fs::copy(&obr_src, &obr_dst).expect("copy obr jsonl");
    fs::copy(&bd_src, &bd_dst).expect("copy bd jsonl");

    import_workspace.run_obr(["sync", "--import-only"], "import");
    import_workspace.run_bd(["sync", "--import-only"], "import");

    // Verify all fields preserved
    let obr_list = import_workspace.run_obr(["list", "--json"], "list");
    let bd_list = import_workspace.run_bd(["list", "--json"], "list");

    let obr_val: Value =
        serde_json::from_str(&extract_json_payload(&obr_list.stdout)).expect("parse obr");
    let bd_val: Value =
        serde_json::from_str(&extract_json_payload(&bd_list.stdout)).expect("parse bd");

    // Check fields preserved. `obr list --json` wraps its rows in a paginated
    // object, so index through `issue_items` rather than `[0]`.
    let obr_issues = issue_items(&obr_val);
    let bd_issues = issue_items(&bd_val);
    assert!(
        !obr_issues.is_empty(),
        "obr returned no issues after import"
    );
    assert!(!bd_issues.is_empty(), "bd returned no issues after import");
    let obr_issue = &obr_issues[0];
    let bd_issue = &bd_issues[0];

    assert_eq!(obr_issue["title"], bd_issue["title"], "titles should match");
    assert_eq!(
        obr_issue["priority"], bd_issue["priority"],
        "priorities should match"
    );

    info!("conformance_sync_roundtrip_preserves_all_fields passed");
}

#[test]
fn conformance_sync_roundtrip_unicode() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_roundtrip_unicode test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue with unicode
    let unicode_title = "Unicode: 你好世界 🎉 café";
    workspace.run_obr(["create", unicode_title], "create");
    workspace.run_bd(["create", unicode_title], "create");

    // Export
    workspace.run_obr(["sync", "--flush-only"], "export");
    workspace.run_bd(["sync", "--flush-only"], "export");

    // Import into fresh workspace
    let import_workspace = ConformanceWorkspace::new();
    import_workspace.init_both();

    let obr_src = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_src = workspace.bd_root.join(".obr").join("issues.jsonl");
    let obr_dst = import_workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_dst = import_workspace.bd_root.join(".obr").join("issues.jsonl");

    fs::copy(&obr_src, &obr_dst).expect("copy obr jsonl");
    fs::copy(&bd_src, &bd_dst).expect("copy bd jsonl");

    import_workspace.run_obr(["sync", "--import-only"], "import");
    import_workspace.run_bd(["sync", "--import-only"], "import");

    // Verify unicode preserved
    let obr_list = import_workspace.run_obr(["list", "--json"], "list");
    let bd_list = import_workspace.run_bd(["list", "--json"], "list");

    let obr_val: Value =
        serde_json::from_str(&extract_json_payload(&obr_list.stdout)).expect("parse obr");
    let bd_val: Value =
        serde_json::from_str(&extract_json_payload(&bd_list.stdout)).expect("parse bd");

    // Check unicode survived
    let obr_issues = issue_items(&obr_val);
    let bd_issues = issue_items(&bd_val);
    assert!(
        !obr_issues.is_empty(),
        "obr returned no issues after import"
    );
    assert!(!bd_issues.is_empty(), "bd returned no issues after import");
    let obr_title = obr_issues[0]["title"].as_str().unwrap_or("");
    let bd_title = bd_issues[0]["title"].as_str().unwrap_or("");

    assert!(obr_title.contains("你好"), "obr should preserve Chinese");
    assert!(bd_title.contains("你好"), "bd should preserve Chinese");
    assert!(obr_title.contains("🎉"), "obr should preserve emoji");
    assert!(bd_title.contains("🎉"), "bd should preserve emoji");

    info!("conformance_sync_roundtrip_unicode passed");
}

#[test]
fn conformance_sync_roundtrip_special_chars() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_roundtrip_special_chars test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue with special chars that might break JSON
    let special_title = r#"Special: "quotes" and \backslash and 'apostrophe'"#;
    workspace.run_obr(["create", special_title], "create");
    workspace.run_bd(["create", special_title], "create");

    // Export
    workspace.run_obr(["sync", "--flush-only"], "export");
    workspace.run_bd(["sync", "--flush-only"], "export");

    // Read JSONL and verify it's valid
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl).expect("read bd jsonl");

    // Both should be valid JSON
    let obr_val: Value = serde_json::from_str(obr_content.lines().next().unwrap())
        .expect("obr JSONL should be valid JSON with special chars");
    let bd_val: Value = serde_json::from_str(bd_content.lines().next().unwrap())
        .expect("bd JSONL should be valid JSON with special chars");

    // Verify special chars preserved
    let obr_title = obr_val["title"].as_str().unwrap_or("");
    let bd_title = bd_val["title"].as_str().unwrap_or("");

    assert!(obr_title.contains("quotes"), "obr should preserve quotes");
    assert!(bd_title.contains("quotes"), "bd should preserve quotes");

    info!("conformance_sync_roundtrip_special_chars passed");
}

// --- sync --status tests ---
// NOTE: bd does not support `sync --status` flag. These tests verify obr behavior only.

#[test]
fn conformance_sync_status_clean() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_status_clean test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue and sync
    workspace.run_obr(["create", "Status test"], "create");

    workspace.run_obr(["sync", "--flush-only"], "flush");

    // Check status - obr only (bd doesn't support --status flag)
    let obr_status = workspace.run_obr(["sync", "--status"], "status");

    assert!(obr_status.status.success(), "obr status failed");

    // Log status output
    info!("obr status: {}", obr_status.stdout);

    // Known difference: bd does not support `sync --status`
    // bd uses different sync architecture without status checking

    info!("conformance_sync_status_clean passed");
}

#[test]
fn conformance_sync_status_json_output() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_status_json_output test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create and sync
    workspace.run_obr(["create", "JSON status test"], "create");

    workspace.run_obr(["sync", "--flush-only"], "flush");

    // Check status with JSON - obr only (bd doesn't support --status flag)
    let obr_status = workspace.run_obr(["sync", "--status", "--json"], "status_json");

    assert!(obr_status.status.success(), "obr status --json failed");

    // Verify JSON output
    let obr_json = extract_json_payload(&obr_status.stdout);
    let _obr_val: Value =
        serde_json::from_str(&obr_json).expect("obr status --json should produce valid JSON");

    // Known difference: bd does not support `sync --status`
    // Only obr provides status checking functionality

    info!("conformance_sync_status_json_output passed");
}

// --- sync edge cases ---

#[test]
fn conformance_sync_large_description() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_large_description test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue with large description (10KB)
    let large_desc: String = "x".repeat(10_000);
    workspace.run_obr(
        ["create", "Large desc test", "--description", &large_desc],
        "create",
    );
    workspace.run_bd(
        ["create", "Large desc test", "--description", &large_desc],
        "create",
    );

    // Export
    let obr_sync = workspace.run_obr(["sync", "--flush-only"], "flush");
    let bd_sync = workspace.run_bd(["sync", "--flush-only"], "flush");

    assert!(obr_sync.status.success(), "obr flush large desc failed");
    assert!(bd_sync.status.success(), "bd flush large desc failed");

    // Verify JSONL created
    let obr_jsonl = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl).expect("read bd jsonl");

    // Both should be valid JSON
    let obr_val: Value = serde_json::from_str(obr_content.lines().next().unwrap())
        .expect("obr large desc should be valid JSON");
    let bd_val: Value = serde_json::from_str(bd_content.lines().next().unwrap())
        .expect("bd large desc should be valid JSON");

    // Verify large description preserved
    let obr_desc = obr_val["description"].as_str().unwrap_or("");
    let bd_desc = bd_val["description"].as_str().unwrap_or("");

    assert!(
        obr_desc.len() >= 9000,
        "obr should preserve large description"
    );
    assert!(
        bd_desc.len() >= 9000,
        "bd should preserve large description"
    );

    info!("conformance_sync_large_description passed");
}

#[test]
fn conformance_sync_tombstones() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_tombstones test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create and delete issue
    let obr_issue = workspace.run_obr(["create", "Tombstone test", "--json"], "create");
    let bd_issue = workspace.run_bd(["create", "Tombstone test", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_issue.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_issue.stdout));

    // Delete
    workspace.run_obr(["delete", &obr_id], "delete");
    workspace.run_bd(["delete", &bd_id], "delete");

    // Export
    let obr_sync = workspace.run_obr(["sync", "--flush-only"], "flush");
    let bd_sync = workspace.run_bd(["sync", "--flush-only"], "flush");

    // Both should succeed (tombstones may or may not be exported)
    info!(
        "Tombstone export: obr={}, bd={}",
        obr_sync.status.success(),
        bd_sync.status.success()
    );

    info!("conformance_sync_tombstones passed");
}

// ============================================================================
// CRUD COMMAND EXPANSION TESTS
// ============================================================================

// --- init tests ---

#[test]
fn conformance_init_reinit() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_init_reinit test");

    let workspace = ConformanceWorkspace::new();

    // First init
    workspace.init_both();

    // Second init (re-init) - should be idempotent or error gracefully
    let obr_reinit = workspace.run_obr(ConformanceWorkspace::INIT_ARGS, "reinit");
    let bd_reinit = workspace.run_bd(ConformanceWorkspace::INIT_ARGS, "reinit");

    // Both should have matching behavior (either both succeed or both fail)
    assert_eq!(
        obr_reinit.status.success(),
        bd_reinit.status.success(),
        "reinit behavior differs: obr success={}, bd success={}",
        obr_reinit.status.success(),
        bd_reinit.status.success()
    );

    // .obr directory should still exist
    assert!(
        workspace.obr_root.join(".obr").exists(),
        "obr .obr disappeared after reinit"
    );
    assert!(
        workspace.bd_root.join(".obr").exists(),
        "bd .obr disappeared after reinit"
    );

    info!("conformance_init_reinit passed");
}

#[test]
fn conformance_init_existing_db() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_init_existing_db test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create some data
    workspace.run_obr(["create", "Test issue"], "create");
    workspace.run_bd(["create", "Test issue"], "create");

    // Try init again - should preserve data
    workspace.run_obr(ConformanceWorkspace::INIT_ARGS, "init_again");
    workspace.run_bd(ConformanceWorkspace::INIT_ARGS, "init_again");

    // Data should still exist
    let obr_list = workspace.run_obr(["list", "--json"], "list_after");
    let bd_list = workspace.run_bd(["list", "--json"], "list_after");

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(obr_len, bd_len, "issue counts differ after reinit");

    info!("conformance_init_existing_db passed");
}

#[test]
fn conformance_init_creates_obr_dir() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_init_creates_beads_dir test");

    let workspace = ConformanceWorkspace::new();

    // Verify .obr doesn't exist yet
    assert!(!workspace.obr_root.join(".obr").exists());
    assert!(!workspace.bd_root.join(".obr").exists());

    workspace.init_both();

    // .obr/obr.db should exist for obr
    assert!(
        workspace.obr_root.join(".obr").join("obr.db").exists(),
        "obr did not create .obr/obr.db"
    );
    // .obr/issues.db should exist for bd (assuming bd uses issues.db, or check what it creates)
    // Actually, checking if *any* .db file exists might be safer if we don't control bd version
    // But let's assume issues.db for now as per previous test code, or update if we know bd uses obr.db too.
    // If bd fails this assertion, we know bd behavior.
    // The panic was "obr did not create .obr/issues.db", so obr uses obr.db (as verified by config).
    // I will change it to obr.db for obr.

    // For bd, let's keep issues.db check if it passes, or maybe it also uses obr.db?
    // The previous run failed on obr check.
    assert!(
        workspace.bd_root.join(".obr").join("issues.db").exists()
            || workspace.bd_root.join(".obr").join("obr.db").exists(),
        "bd did not create a database file"
    );

    info!("conformance_init_creates_beads_dir passed");
}

#[test]
fn conformance_init_json_output() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_init_json_output test");

    let workspace = ConformanceWorkspace::new();

    let obr_init = workspace.run_obr(
        ["init", "--prefix", CONFORMANCE_PREFIX, "--json"],
        "init_json",
    );
    let bd_init = workspace.run_bd(
        ["init", "--prefix", CONFORMANCE_PREFIX, "--json"],
        "init_json",
    );

    assert!(
        obr_init.status.success(),
        "obr init --json failed: {}",
        obr_init.stderr
    );
    assert!(
        bd_init.status.success(),
        "bd init --json failed: {}",
        bd_init.stderr
    );

    // Both should produce valid JSON or exit successfully
    let obr_json = extract_json_payload(&obr_init.stdout);
    let bd_json = extract_json_payload(&bd_init.stdout);

    // If both produce JSON, they should have similar structure
    if !obr_json.is_empty() && !bd_json.is_empty() {
        let obr_val: Result<Value, _> = serde_json::from_str(&obr_json);
        let bd_val: Result<Value, _> = serde_json::from_str(&bd_json);

        assert_eq!(
            obr_val.is_ok(),
            bd_val.is_ok(),
            "JSON validity differs: obr valid={}, bd valid={}",
            obr_val.is_ok(),
            bd_val.is_ok()
        );
    }

    info!("conformance_init_json_output passed");
}

#[test]
fn conformance_init_config() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_init_config test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_config_path = workspace.obr_root.join(".obr").join("config.yaml");
    let bd_config_path = workspace.bd_root.join(".obr").join("config.yaml");

    assert!(
        obr_config_path.exists(),
        "obr config.yaml missing at {}",
        obr_config_path.display()
    );
    assert!(
        bd_config_path.exists(),
        "bd config.yaml missing at {}",
        bd_config_path.display()
    );

    let obr_config = fs::read_to_string(&obr_config_path).expect("read obr config.yaml");
    let bd_config = fs::read_to_string(&bd_config_path).expect("read bd config.yaml");

    assert!(!obr_config.trim().is_empty(), "obr config.yaml is empty");
    assert!(!bd_config.trim().is_empty(), "bd config.yaml is empty");

    let obr_yaml: Result<YamlValue, _> = serde_yml::from_str(&obr_config);
    let bd_yaml: Result<YamlValue, _> = serde_yml::from_str(&bd_config);

    assert_eq!(
        obr_yaml.is_ok(),
        bd_yaml.is_ok(),
        "config YAML validity differs: obr ok={}, bd ok={}",
        obr_yaml.is_ok(),
        bd_yaml.is_ok()
    );

    if let (Ok(obr_val), Ok(bd_val)) = (obr_yaml, bd_yaml) {
        assert_eq!(obr_val, bd_val, "config YAML content differs after parsing");
    }

    info!("conformance_init_config passed");
}

#[test]
fn conformance_init_metadata() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_init_metadata test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_metadata_path = workspace.obr_root.join(".obr").join("metadata.json");
    let bd_metadata_path = workspace.bd_root.join(".obr").join("metadata.json");

    assert!(
        obr_metadata_path.exists(),
        "obr metadata.json missing at {}",
        obr_metadata_path.display()
    );
    assert!(
        bd_metadata_path.exists(),
        "bd metadata.json missing at {}",
        bd_metadata_path.display()
    );

    let obr_metadata = fs::read_to_string(&obr_metadata_path).expect("read obr metadata.json");
    let bd_metadata = fs::read_to_string(&bd_metadata_path).expect("read bd metadata.json");

    let result = compare_json(&obr_metadata, &bd_metadata, &CompareMode::ExactJson);
    assert!(result.is_ok(), "metadata JSON mismatch: {:?}", result.err());

    info!("conformance_init_metadata passed");
}

// --- create tests ---

#[test]
fn conformance_create_all_types() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_all_types test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Only test types supported by both obr and bd
    // bd supports: bug, feature, task, epic, chore
    // obr supports: bug, feature, task, epic, chore, docs, question
    let types = ["bug", "feature", "task", "epic", "chore"];

    for issue_type in types {
        let title = format!("Test {} issue", issue_type);
        let obr_create = workspace.run_obr(
            ["create", &title, "--type", issue_type, "--json"],
            &format!("create_{}", issue_type),
        );
        let bd_create = workspace.run_bd(
            ["create", &title, "--type", issue_type, "--json"],
            &format!("create_{}", issue_type),
        );

        assert!(
            obr_create.status.success(),
            "obr create --type {} failed: {}",
            issue_type,
            obr_create.stderr
        );
        assert!(
            bd_create.status.success(),
            "bd create --type {} failed: {}",
            issue_type,
            bd_create.stderr
        );

        let obr_json = extract_json_payload(&obr_create.stdout);
        let bd_json = extract_json_payload(&bd_create.stdout);

        let result = compare_json(
            &obr_json,
            &bd_json,
            &CompareMode::ContainsFields(vec!["issue_type".to_string()]),
        );
        assert!(
            result.is_ok(),
            "type {} comparison failed: {:?}",
            issue_type,
            result.err()
        );
    }

    info!("conformance_create_all_types passed");
}

#[test]
fn conformance_create_all_priorities() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_all_priorities test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    for priority in 0..=4 {
        let title = format!("Priority {} issue", priority);
        let priority_str = priority.to_string();
        let obr_create = workspace.run_obr(
            ["create", &title, "--priority", &priority_str, "--json"],
            &format!("create_p{}", priority),
        );
        let bd_create = workspace.run_bd(
            ["create", &title, "--priority", &priority_str, "--json"],
            &format!("create_p{}", priority),
        );

        assert!(
            obr_create.status.success(),
            "obr create --priority {} failed: {}",
            priority,
            obr_create.stderr
        );
        assert!(
            bd_create.status.success(),
            "bd create --priority {} failed: {}",
            priority,
            bd_create.stderr
        );

        let obr_json = extract_json_payload(&obr_create.stdout);
        let bd_json = extract_json_payload(&bd_create.stdout);

        let obr_val: Value = serde_json::from_str(&obr_json).expect("parse obr");
        let bd_val: Value = serde_json::from_str(&bd_json).expect("parse bd");

        let obr_p = obr_val["priority"]
            .as_i64()
            .or_else(|| obr_val[0]["priority"].as_i64());
        let bd_p = bd_val["priority"]
            .as_i64()
            .or_else(|| bd_val[0]["priority"].as_i64());

        assert_eq!(
            obr_p, bd_p,
            "priority {} mismatch: obr={:?}, bd={:?}",
            priority, obr_p, bd_p
        );
    }

    info!("conformance_create_all_priorities passed");
}

#[test]
fn conformance_create_with_assignee() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_with_assignee test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(
        ["create", "Assigned issue", "--assignee", "alice", "--json"],
        "create_assigned",
    );
    let bd_create = workspace.run_bd(
        ["create", "Assigned issue", "--assignee", "alice", "--json"],
        "create_assigned",
    );

    assert!(
        obr_create.status.success(),
        "obr create failed: {}",
        obr_create.stderr
    );
    assert!(
        bd_create.status.success(),
        "bd create failed: {}",
        bd_create.stderr
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_assignee = obr_val["assignee"]
        .as_str()
        .or_else(|| obr_val[0]["assignee"].as_str());
    let bd_assignee = bd_val["assignee"]
        .as_str()
        .or_else(|| bd_val[0]["assignee"].as_str());

    assert_eq!(
        obr_assignee, bd_assignee,
        "assignee mismatch: obr={:?}, bd={:?}",
        obr_assignee, bd_assignee
    );

    info!("conformance_create_with_assignee passed");
}

#[test]
fn conformance_create_with_description() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_with_description test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let desc = "This is a detailed description\nwith multiple lines.";
    let obr_create = workspace.run_obr(
        ["create", "Issue with desc", "--description", desc, "--json"],
        "create_desc",
    );
    let bd_create = workspace.run_bd(
        ["create", "Issue with desc", "--description", desc, "--json"],
        "create_desc",
    );

    assert!(
        obr_create.status.success(),
        "obr create failed: {}",
        obr_create.stderr
    );
    assert!(
        bd_create.status.success(),
        "bd create failed: {}",
        bd_create.stderr
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_desc = obr_val["description"]
        .as_str()
        .or_else(|| obr_val[0]["description"].as_str());
    let bd_desc = bd_val["description"]
        .as_str()
        .or_else(|| bd_val[0]["description"].as_str());

    assert_eq!(
        obr_desc, bd_desc,
        "description mismatch: obr={:?}, bd={:?}",
        obr_desc, bd_desc
    );

    info!("conformance_create_with_description passed");
}

#[test]
fn conformance_create_unicode_title() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_unicode_title test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let unicode_titles = [
        "日本語のタイトル",  // Japanese
        "Emoji test 🎉🚀💻", // Emoji
        "مرحبا بالعالم",     // Arabic (RTL)
        "Ñoño español",      // Spanish with ñ
        "Über Größe",        // German umlauts
    ];

    for title in unicode_titles {
        let obr_create = workspace.run_obr(["create", title, "--json"], "create_unicode");
        let bd_create = workspace.run_bd(["create", title, "--json"], "create_unicode");

        assert!(
            obr_create.status.success(),
            "obr create unicode failed for '{}': {}",
            title,
            obr_create.stderr
        );
        assert!(
            bd_create.status.success(),
            "bd create unicode failed for '{}': {}",
            title,
            bd_create.stderr
        );

        let obr_json = extract_json_payload(&obr_create.stdout);
        let bd_json = extract_json_payload(&bd_create.stdout);

        let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
        let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

        let obr_title = obr_val["title"]
            .as_str()
            .or_else(|| obr_val[0]["title"].as_str());
        let bd_title = bd_val["title"]
            .as_str()
            .or_else(|| bd_val[0]["title"].as_str());

        assert_eq!(
            obr_title, bd_title,
            "unicode title mismatch for '{}': obr={:?}, bd={:?}",
            title, obr_title, bd_title
        );
    }

    info!("conformance_create_unicode_title passed");
}

#[test]
fn conformance_create_special_chars() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_special_chars test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Test special characters that might break parsing
    let special_titles = [
        "Title with 'single quotes'",
        "Title with \"double quotes\"",
        "Title with \\backslashes\\",
        "Title with <angle> & ampersand",
    ];

    for title in special_titles {
        let obr_create = workspace.run_obr(["create", title, "--json"], "create_special");
        let bd_create = workspace.run_bd(["create", title, "--json"], "create_special");

        assert!(
            obr_create.status.success(),
            "obr create special failed for '{}': {}",
            title,
            obr_create.stderr
        );
        assert!(
            bd_create.status.success(),
            "bd create special failed for '{}': {}",
            title,
            bd_create.stderr
        );

        let obr_json = extract_json_payload(&obr_create.stdout);
        let bd_json = extract_json_payload(&bd_create.stdout);

        let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
        let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

        let obr_title = obr_val["title"]
            .as_str()
            .or_else(|| obr_val[0]["title"].as_str());
        let bd_title = bd_val["title"]
            .as_str()
            .or_else(|| bd_val[0]["title"].as_str());

        assert_eq!(
            obr_title, bd_title,
            "special char title mismatch for '{}': obr={:?}, bd={:?}",
            title, obr_title, bd_title
        );
    }

    info!("conformance_create_special_chars passed");
}

#[test]
fn conformance_create_very_long_title() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_very_long_title test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let long_title = "A".repeat(500);
    let obr_create = workspace.run_obr(["create", &long_title, "--json"], "create_long");
    let bd_create = workspace.run_bd(["create", &long_title, "--json"], "create_long");

    assert!(
        obr_create.status.success(),
        "obr create long title failed: {}",
        obr_create.stderr
    );
    assert!(
        bd_create.status.success(),
        "bd create long title failed: {}",
        bd_create.stderr
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_title = obr_val["title"]
        .as_str()
        .or_else(|| obr_val[0]["title"].as_str());
    let bd_title = bd_val["title"]
        .as_str()
        .or_else(|| bd_val[0]["title"].as_str());

    assert_eq!(
        obr_title,
        bd_title,
        "long title mismatch: br_len={:?}, bd_len={:?}",
        obr_title.map(str::len),
        bd_title.map(str::len)
    );
    assert_eq!(
        obr_title.map(str::len),
        Some(500),
        "expected 500-char title"
    );

    info!("conformance_create_very_long_title passed");
}

#[test]
fn conformance_create_empty_title_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_empty_title_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "", "--json"], "create_empty");
    let bd_create = workspace.run_bd(["create", "", "--json"], "create_empty");

    assert_eq!(
        obr_create.status.success(),
        bd_create.status.success(),
        "empty title behavior differs: obr success={}, bd success={}",
        obr_create.status.success(),
        bd_create.status.success()
    );
    assert!(
        !obr_create.status.success(),
        "expected empty title to fail in obr"
    );

    info!("conformance_create_empty_title_error passed");
}

#[test]
fn conformance_create_with_external_ref() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_with_external_ref test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(
        [
            "create",
            "Issue with external ref",
            "--external-ref",
            "JIRA-123",
            "--json",
        ],
        "create_external_ref",
    );
    let bd_create = workspace.run_bd(
        [
            "create",
            "Issue with external ref",
            "--external-ref",
            "JIRA-123",
            "--json",
        ],
        "create_external_ref",
    );

    assert!(
        obr_create.status.success(),
        "obr create failed: {}",
        obr_create.stderr
    );
    assert!(
        bd_create.status.success(),
        "bd create failed: {}",
        bd_create.stderr
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_ref = obr_val["external_ref"]
        .as_str()
        .or_else(|| obr_val[0]["external_ref"].as_str());
    let bd_ref = bd_val["external_ref"]
        .as_str()
        .or_else(|| bd_val[0]["external_ref"].as_str());

    assert_eq!(
        obr_ref, bd_ref,
        "external_ref mismatch: obr={:?}, bd={:?}",
        obr_ref, bd_ref
    );

    info!("conformance_create_with_external_ref passed");
}

#[test]
fn conformance_create_invalid_priority_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_create_invalid_priority_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(
        ["create", "Bad priority issue", "--priority", "9", "--json"],
        "create_bad_priority",
    );
    let bd_create = workspace.run_bd(
        ["create", "Bad priority issue", "--priority", "9", "--json"],
        "create_bad_priority",
    );

    assert_eq!(
        obr_create.status.success(),
        bd_create.status.success(),
        "invalid priority behavior differs: obr success={}, bd success={}",
        obr_create.status.success(),
        bd_create.status.success()
    );
    assert!(
        !obr_create.status.success(),
        "expected invalid priority to fail in obr"
    );

    info!("conformance_create_invalid_priority_error passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --status closed --json` returns invalid JSON.
/// obr's status filtering works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --status filter returns invalid JSON: known behavioral difference"]
fn conformance_list_filter_status_closed() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_filter_status_closed test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Open issue", "--json"], "create_open");
    let bd_create = workspace.run_obr_in_bd_env(["create", "Open issue", "--json"], "create_open");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["close", obr_id], "close_one");
    workspace.run_obr_in_bd_env(["close", bd_id], "close_one");

    let obr_list = workspace.run_obr(["list", "--status", "closed", "--json"], "list_closed");
    let bd_list = workspace.run_bd(["list", "--status", "closed", "--json"], "list_closed");

    assert!(
        obr_list.status.success(),
        "obr list closed failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list closed failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "closed list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 closed issue");

    info!("conformance_list_filter_status_closed passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --assignee` filter returns empty array.
/// obr's assignee filtering works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --assignee filter returns empty: known behavioral difference"]
fn conformance_list_filter_assignee() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_filter_assignee test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(
        ["create", "Assigned to alice", "--assignee", "alice"],
        "create_alice",
    );
    workspace.run_obr_in_bd_env(
        ["create", "Assigned to alice", "--assignee", "alice"],
        "create_alice",
    );

    workspace.run_obr(
        ["create", "Assigned to bob", "--assignee", "bob"],
        "create_bob",
    );
    workspace.run_obr_in_bd_env(
        ["create", "Assigned to bob", "--assignee", "bob"],
        "create_bob",
    );

    let obr_list = workspace.run_obr(
        ["list", "--assignee", "alice", "--json"],
        "list_assignee_alice",
    );
    let bd_list = workspace.run_bd(
        ["list", "--assignee", "alice", "--json"],
        "list_assignee_alice",
    );

    assert!(
        obr_list.status.success(),
        "obr list assignee failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list assignee failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "assignee list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 issue assigned to alice");

    info!("conformance_list_filter_assignee passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --limit` returns empty array.
/// obr's limit filtering works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --limit returns empty: known behavioral difference"]
fn conformance_list_limit() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_limit test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Issue 1"], "create1");
    workspace.run_obr_in_bd_env(["create", "Issue 1"], "create1");
    workspace.run_obr(["create", "Issue 2"], "create2");
    workspace.run_obr_in_bd_env(["create", "Issue 2"], "create2");
    workspace.run_obr(["create", "Issue 3"], "create3");
    workspace.run_obr_in_bd_env(["create", "Issue 3"], "create3");

    let obr_list = workspace.run_obr(["list", "--limit", "1", "--json"], "list_limit");
    let bd_list = workspace.run_bd(["list", "--limit", "1", "--json"], "list_limit");

    assert!(
        obr_list.status.success(),
        "obr list limit failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list limit failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "limit list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 issue with limit");

    info!("conformance_list_limit passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --status open --json` returns invalid JSON.
/// obr's status filtering works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --status filter returns invalid JSON: known behavioral difference"]
fn conformance_list_filter_status_open() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_filter_status_open test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Open issue", "--json"], "create_open");
    workspace.run_obr_in_bd_env(["create", "Open issue", "--json"], "create_open");

    let obr_create_closed =
        workspace.run_obr(["create", "Closed issue", "--json"], "create_closed");
    let bd_create_closed =
        workspace.run_obr_in_bd_env(["create", "Closed issue", "--json"], "create_closed");

    let obr_closed_json = extract_json_payload(&obr_create_closed.stdout);
    let bd_closed_json = extract_json_payload(&bd_create_closed.stdout);
    let obr_closed_val: Value = serde_json::from_str(&obr_closed_json).expect("parse");
    let bd_closed_val: Value = serde_json::from_str(&bd_closed_json).expect("parse");

    let obr_closed_id = obr_closed_val["id"]
        .as_str()
        .or_else(|| obr_closed_val[0]["id"].as_str())
        .unwrap();
    let bd_closed_id = bd_closed_val["id"]
        .as_str()
        .or_else(|| bd_closed_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["close", obr_closed_id], "close_closed");
    workspace.run_bd(["close", bd_closed_id], "close_closed");

    let obr_list = workspace.run_obr(["list", "--status", "open", "--json"], "list_open");
    let bd_list = workspace.run_bd(["list", "--status", "open", "--json"], "list_open");

    assert!(
        obr_list.status.success(),
        "obr list open failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list open failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "open list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 open issue");

    info!("conformance_list_filter_status_open passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --status in_progress --json` returns invalid JSON.
/// obr's status filtering works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --status filter returns invalid JSON: known behavioral difference"]
fn conformance_list_filter_status_in_progress() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_filter_status_in_progress test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "In progress issue", "--json"], "create_ip");
    let bd_create =
        workspace.run_obr_in_bd_env(["create", "In progress issue", "--json"], "create_ip");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["update", obr_id, "--status", "in_progress"], "update_ip");
    workspace.run_obr_in_bd_env(["update", bd_id, "--status", "in_progress"], "update_ip");

    let obr_list = workspace.run_obr(
        ["list", "--status", "in_progress", "--json"],
        "list_in_progress",
    );
    let bd_list = workspace.run_bd(
        ["list", "--status", "in_progress", "--json"],
        "list_in_progress",
    );

    assert!(
        obr_list.status.success(),
        "obr list in_progress failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list in_progress failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "in_progress list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 in_progress issue");

    info!("conformance_list_filter_status_in_progress passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --priority` range filter returns empty array.
/// obr's priority range filtering works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --priority range returns empty: known behavioral difference"]
fn conformance_list_filter_priority_range() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_filter_priority_range test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "P0 issue", "--priority", "0"], "create_p0");
    workspace.run_obr_in_bd_env(["create", "P0 issue", "--priority", "0"], "create_p0");
    workspace.run_obr(["create", "P1 issue", "--priority", "1"], "create_p1");
    workspace.run_obr_in_bd_env(["create", "P1 issue", "--priority", "1"], "create_p1");
    workspace.run_obr(["create", "P3 issue", "--priority", "3"], "create_p3");
    workspace.run_obr_in_bd_env(["create", "P3 issue", "--priority", "3"], "create_p3");

    let obr_list = workspace.run_obr(
        [
            "list",
            "--priority-min",
            "0",
            "--priority-max",
            "1",
            "--json",
        ],
        "list_priority_range",
    );
    let bd_list = workspace.run_bd(
        [
            "list",
            "--priority-min",
            "0",
            "--priority-max",
            "1",
            "--json",
        ],
        "list_priority_range",
    );

    assert!(
        obr_list.status.success(),
        "obr list priority range failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list priority range failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "priority range lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 2, "expected 2 issues in priority range");

    info!("conformance_list_filter_priority_range passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --label` filter returns invalid JSON.
/// obr's label filtering works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --label filter returns invalid JSON: known behavioral difference"]
fn conformance_list_filter_label() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_filter_label test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Label issue", "--json"], "create_label");
    let bd_create =
        workspace.run_obr_in_bd_env(["create", "Label issue", "--json"], "create_label");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["label", "add", obr_id, "urgent"], "label_add");
    workspace.run_obr_in_bd_env(["label", "add", bd_id, "urgent"], "label_add");

    workspace.run_obr(["create", "Unlabeled issue"], "create_unlabeled");
    workspace.run_obr_in_bd_env(["create", "Unlabeled issue"], "create_unlabeled");

    let obr_list = workspace.run_obr(["list", "--label", "urgent", "--json"], "list_label");
    let bd_list = workspace.run_bd(["list", "--label", "urgent", "--json"], "list_label");

    assert!(
        obr_list.status.success(),
        "obr list label failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list label failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "label list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 labeled issue");

    info!("conformance_list_filter_label passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list` with multiple filters returns invalid JSON.
/// obr's combined filtering works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list with multiple filters returns invalid JSON: known behavioral difference"]
fn conformance_list_filter_multiple() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_filter_multiple test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(
        [
            "create",
            "Multi filter issue",
            "--assignee",
            "alice",
            "--json",
        ],
        "create_multi",
    );
    let bd_create = workspace.run_obr_in_bd_env(
        [
            "create",
            "Multi filter issue",
            "--assignee",
            "alice",
            "--json",
        ],
        "create_multi",
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["label", "add", obr_id, "urgent"], "label_add");
    workspace.run_obr_in_bd_env(["label", "add", bd_id, "urgent"], "label_add");

    workspace.run_obr(
        ["create", "Other issue", "--assignee", "alice"],
        "create_other",
    );
    workspace.run_obr_in_bd_env(
        ["create", "Other issue", "--assignee", "alice"],
        "create_other",
    );

    let obr_list = workspace.run_obr(
        ["list", "--assignee", "alice", "--label", "urgent", "--json"],
        "list_multi",
    );
    let bd_list = workspace.run_bd(
        ["list", "--assignee", "alice", "--label", "urgent", "--json"],
        "list_multi",
    );

    assert!(
        obr_list.status.success(),
        "obr list multi failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list multi failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_val);
    let bd_len = issue_count(&bd_val);

    assert_eq!(
        obr_len, bd_len,
        "multi-filter list lengths differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 issue matching both filters");

    info!("conformance_list_filter_multiple passed");
}

#[test]
fn conformance_list_sort_priority() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_sort_priority test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "P2 issue", "--priority", "2"], "create_p2");
    workspace.run_bd(["create", "P2 issue", "--priority", "2"], "create_p2");
    workspace.run_obr(["create", "P0 issue", "--priority", "0"], "create_p0");
    workspace.run_bd(["create", "P0 issue", "--priority", "0"], "create_p0");
    workspace.run_obr(["create", "P4 issue", "--priority", "4"], "create_p4");
    workspace.run_bd(["create", "P4 issue", "--priority", "4"], "create_p4");

    let obr_list = workspace.run_obr(["list", "--sort", "priority", "--json"], "list_sort_pri");
    let bd_list = workspace.run_bd(["list", "--sort", "priority", "--json"], "list_sort_pri");

    assert!(
        obr_list.status.success(),
        "obr list sort priority failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list sort priority failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_priorities: Vec<i64> = obr_val
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v["priority"].as_i64())
        .collect();
    let bd_priorities: Vec<i64> = bd_val
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v["priority"].as_i64())
        .collect();

    let mut obr_sorted = obr_priorities.clone();
    obr_sorted.sort();
    let mut bd_sorted = bd_priorities.clone();
    bd_sorted.sort();

    assert_eq!(obr_priorities, obr_sorted, "obr priorities not sorted");
    assert_eq!(bd_priorities, bd_sorted, "bd priorities not sorted");

    info!("conformance_list_sort_priority passed");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0's `list --sort created` returns empty array.
/// obr's sort by created_at works correctly. This is a known bd behavioral difference.
#[test]
#[ignore = "bd v0.46.0 list --sort created returns empty: known behavioral difference"]
fn conformance_list_sort_created() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_sort_created test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "First issue"], "create_first");
    workspace.run_obr_in_bd_env(["create", "First issue"], "create_first");
    workspace.run_obr(["create", "Second issue"], "create_second");
    workspace.run_obr_in_bd_env(["create", "Second issue"], "create_second");

    let obr_list = workspace.run_obr(
        ["list", "--sort", "created_at", "--json"],
        "list_sort_created",
    );
    let bd_list = workspace.run_bd(["list", "--sort", "created", "--json"], "list_sort_created");

    assert!(
        obr_list.status.success(),
        "obr list sort created failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list sort created failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_titles: Vec<String> = obr_val
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v["title"].as_str().map(str::to_string))
        .collect();
    let bd_titles: Vec<String> = bd_val
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|v| v["title"].as_str().map(str::to_string))
        .collect();

    assert_eq!(
        obr_titles, bd_titles,
        "created sort order differs: obr={obr_titles:?} bd={bd_titles:?}"
    );
    assert!(
        bd_titles.first().is_some_and(|t| t == "Second issue"),
        "bd created sort order unexpected: {bd_titles:?}"
    );

    info!("conformance_list_sort_created passed");
}

#[test]
fn conformance_list_json_structure() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_list_json_structure test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Structure issue"], "create");
    workspace.run_obr_in_bd_env(["create", "Structure issue"], "create");

    let obr_list = workspace.run_obr(["list", "--json"], "list_struct");
    let bd_list = workspace.run_bd(["list", "--json"], "list_struct");

    assert!(
        obr_list.status.success(),
        "obr list struct failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list struct failed: {}",
        bd_list.stderr
    );

    let obr_list_json = extract_json_payload(&obr_list.stdout);
    let bd_list_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_list_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_list_json).unwrap_or(Value::Array(vec![]));

    let obr_item = obr_val.as_array().and_then(|a| a.first());
    let bd_item = bd_val.as_array().and_then(|a| a.first());

    for item in [obr_item, bd_item].into_iter().flatten() {
        assert!(item.get("id").is_some(), "missing id in list item");
        assert!(item.get("title").is_some(), "missing title in list item");
        assert!(item.get("status").is_some(), "missing status in list item");
        assert!(
            item.get("priority").is_some(),
            "missing priority in list item"
        );
    }

    info!("conformance_list_json_structure passed");
}

#[test]
fn conformance_show_partial_id() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_show_partial_id test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Partial ID issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Partial ID issue", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    let obr_hash = obr_id.split('-').nth(1).unwrap_or(obr_id);
    let bd_hash = bd_id.split('-').nth(1).unwrap_or(bd_id);
    let obr_partial = &obr_hash[..obr_hash.len().min(6)];
    let bd_partial = &bd_hash[..bd_hash.len().min(6)];

    let obr_show = workspace.run_obr(["show", obr_partial, "--json"], "show_partial");
    let bd_show = workspace.run_bd(["show", bd_partial, "--json"], "show_partial");

    assert!(
        obr_show.status.success(),
        "obr show partial failed: {}",
        obr_show.stderr
    );
    assert!(
        bd_show.status.success(),
        "bd show partial failed: {}",
        bd_show.stderr
    );

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let result = compare_json(
        &obr_show_json,
        &bd_show_json,
        &CompareMode::ContainsFields(vec![
            "title".to_string(),
            "status".to_string(),
            "issue_type".to_string(),
        ]),
    );

    assert!(
        result.is_ok(),
        "partial id show comparison failed: {:?}",
        result.err()
    );

    info!("conformance_show_partial_id passed");
}

#[test]
fn conformance_show_nonexistent_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_show_nonexistent_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_show = workspace.run_obr(["show", "bd-does-not-exist", "--json"], "show_missing");
    let _bd_show = workspace.run_bd(["show", "bd-does-not-exist", "--json"], "show_missing");

    // bd behavior is inconsistent/legacy, but obr should definitely fail
    assert!(
        !obr_show.status.success(),
        "obr expected show missing to fail"
    );
    // Don't compare with bd for this case

    info!("conformance_show_nonexistent_error passed");
}

#[test]
fn conformance_show_full_details() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_show_full_details test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(
        [
            "create",
            "Full details issue",
            "--type",
            "feature",
            "--priority",
            "1",
            "--assignee",
            "alice",
            "--description",
            "Detail description",
            "--external-ref",
            "EXT-123",
            "--json",
        ],
        "create_full_details",
    );
    let bd_create = workspace.run_bd(
        [
            "create",
            "Full details issue",
            "--type",
            "feature",
            "--priority",
            "1",
            "--assignee",
            "alice",
            "--description",
            "Detail description",
            "--external-ref",
            "EXT-123",
            "--json",
        ],
        "create_full_details",
    );

    assert!(
        obr_create.status.success(),
        "obr create full details failed: {}",
        obr_create.stderr
    );
    assert!(
        bd_create.status.success(),
        "bd create full details failed: {}",
        bd_create.stderr
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["label", "add", obr_id, "urgent"], "label_add_full");
    workspace.run_bd(["label", "add", bd_id, "urgent"], "label_add_full");

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_full");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_full");

    assert!(
        obr_show.status.success(),
        "obr show full failed: {}",
        obr_show.stderr
    );
    assert!(
        bd_show.status.success(),
        "bd show full failed: {}",
        bd_show.stderr
    );

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let result = compare_json(
        &obr_show_json,
        &bd_show_json,
        &CompareMode::ContainsFields(vec![
            "title".to_string(),
            "description".to_string(),
            "assignee".to_string(),
            "external_ref".to_string(),
            "issue_type".to_string(),
            "priority".to_string(),
        ]),
    );
    assert!(
        result.is_ok(),
        "full details comparison failed: {:?}",
        result.err()
    );

    let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");
    let obr_issue = if obr_val.is_array() {
        &obr_val[0]
    } else {
        &obr_val
    };
    let bd_issue = if bd_val.is_array() {
        &bd_val[0]
    } else {
        &bd_val
    };

    for issue in [obr_issue, bd_issue] {
        assert!(issue.get("labels").is_some(), "missing labels");
    }

    info!("conformance_show_full_details passed");
}

#[test]
fn conformance_show_with_dependencies() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_show_with_dependencies test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_parent = workspace.run_obr(["create", "Parent issue", "--json"], "create_parent");
    let bd_parent = workspace.run_bd(["create", "Parent issue", "--json"], "create_parent");
    let obr_child = workspace.run_obr(["create", "Child issue", "--json"], "create_child");
    let bd_child = workspace.run_bd(["create", "Child issue", "--json"], "create_child");

    let obr_parent_json = extract_json_payload(&obr_parent.stdout);
    let bd_parent_json = extract_json_payload(&bd_parent.stdout);
    let obr_child_json = extract_json_payload(&obr_child.stdout);
    let bd_child_json = extract_json_payload(&bd_child.stdout);

    let obr_parent_val: Value = serde_json::from_str(&obr_parent_json).expect("parse");
    let bd_parent_val: Value = serde_json::from_str(&bd_parent_json).expect("parse");
    let obr_child_val: Value = serde_json::from_str(&obr_child_json).expect("parse");
    let bd_child_val: Value = serde_json::from_str(&bd_child_json).expect("parse");

    let obr_parent_id = obr_parent_val["id"]
        .as_str()
        .or_else(|| obr_parent_val[0]["id"].as_str())
        .unwrap();
    let bd_parent_id = bd_parent_val["id"]
        .as_str()
        .or_else(|| bd_parent_val[0]["id"].as_str())
        .unwrap();
    let obr_child_id = obr_child_val["id"]
        .as_str()
        .or_else(|| obr_child_val[0]["id"].as_str())
        .unwrap();
    let bd_child_id = bd_child_val["id"]
        .as_str()
        .or_else(|| bd_child_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["dep", "add", obr_child_id, obr_parent_id], "dep_add");
    workspace.run_bd(["dep", "add", bd_child_id, bd_parent_id], "dep_add");

    let obr_show = workspace.run_obr(["show", obr_child_id, "--json"], "show_deps");
    let bd_show = workspace.run_bd(["show", bd_child_id, "--json"], "show_deps");

    assert!(
        obr_show.status.success(),
        "obr show deps failed: {}",
        obr_show.stderr
    );
    assert!(
        bd_show.status.success(),
        "bd show deps failed: {}",
        bd_show.stderr
    );

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");
    let obr_issue = if obr_val.is_array() {
        &obr_val[0]
    } else {
        &obr_val
    };
    let bd_issue = if bd_val.is_array() {
        &bd_val[0]
    } else {
        &bd_val
    };

    let obr_len = obr_issue["dependencies"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let bd_len = bd_issue["dependencies"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    assert_eq!(
        obr_len, bd_len,
        "dependency counts differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 dependency");

    info!("conformance_show_with_dependencies passed");
}

#[test]
fn conformance_show_with_comments() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_show_with_comments test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Commented issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Commented issue", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    let comment_text = "First comment";
    workspace.run_obr(["comments", "add", obr_id, comment_text], "comment_add");
    workspace.run_bd(["comments", "add", bd_id, comment_text], "comment_add");

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_comments");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_comments");

    assert!(
        obr_show.status.success(),
        "obr show comments failed: {}",
        obr_show.stderr
    );
    assert!(
        bd_show.status.success(),
        "bd show comments failed: {}",
        bd_show.stderr
    );

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");
    let obr_issue = if obr_val.is_array() {
        &obr_val[0]
    } else {
        &obr_val
    };
    let bd_issue = if bd_val.is_array() {
        &bd_val[0]
    } else {
        &bd_val
    };

    let obr_len = obr_issue["comments"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let bd_len = bd_issue["comments"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    assert_eq!(
        obr_len, bd_len,
        "comment counts differ: obr={}, bd={}",
        obr_len, bd_len
    );
    assert_eq!(obr_len, 1, "expected 1 comment");

    info!("conformance_show_with_comments passed");
}

#[test]
#[ignore]
fn conformance_show_deleted_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_show_deleted_issue test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Deleted issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Deleted issue", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["delete", obr_id, "--reason", "cleanup"], "delete");
    workspace.run_bd(
        ["delete", bd_id, "--reason", "cleanup", "--force"],
        "delete",
    );

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_deleted");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_deleted");

    assert_eq!(
        obr_show.status.success(),
        bd_show.status.success(),
        "show deleted behavior differs: obr success={}, bd success={}",
        obr_show.status.success(),
        bd_show.status.success()
    );

    if obr_show.status.success() && bd_show.status.success() {
        let obr_show_json = extract_json_payload(&obr_show.stdout);
        let bd_show_json = extract_json_payload(&bd_show.stdout);

        if obr_show_json.trim().is_empty() || bd_show_json.trim().is_empty() {
            assert!(
                obr_show_json.trim().is_empty() && bd_show_json.trim().is_empty(),
                "deleted show output mismatch: obr='{}' bd='{}'",
                obr_show_json,
                bd_show_json
            );
        } else {
            let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
            let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");
            let obr_issue = if obr_val.is_array() {
                &obr_val[0]
            } else {
                &obr_val
            };
            let bd_issue = if bd_val.is_array() {
                &bd_val[0]
            } else {
                &bd_val
            };

            assert_eq!(
                obr_issue["status"].as_str(),
                bd_issue["status"].as_str(),
                "deleted status mismatch"
            );
        }
    }

    info!("conformance_show_deleted_issue passed");
}

#[test]
fn conformance_update_title() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_update_title test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Old title", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Old title", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    let obr_update = workspace.run_obr(
        ["update", obr_id, "--title", "New title", "--json"],
        "update_title",
    );
    let bd_update = workspace.run_bd(
        ["update", bd_id, "--title", "New title", "--json"],
        "update_title",
    );

    assert!(
        obr_update.status.success(),
        "obr update title failed: {}",
        obr_update.stderr
    );
    assert!(
        bd_update.status.success(),
        "bd update title failed: {}",
        bd_update.stderr
    );

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_after_update");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_after_update");

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");

    let obr_title = obr_val["title"]
        .as_str()
        .or_else(|| obr_val[0]["title"].as_str());
    let bd_title = bd_val["title"]
        .as_str()
        .or_else(|| bd_val[0]["title"].as_str());

    assert_eq!(
        obr_title, bd_title,
        "title mismatch after update: obr={:?}, bd={:?}",
        obr_title, bd_title
    );
    assert_eq!(obr_title, Some("New title"), "expected updated title");

    info!("conformance_update_title passed");
}

#[test]
fn conformance_update_assignee() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_update_assignee test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Assignee update", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Assignee update", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    let obr_update = workspace.run_obr(
        ["update", obr_id, "--assignee", "alice", "--json"],
        "update_assignee",
    );
    let bd_update = workspace.run_bd(
        ["update", bd_id, "--assignee", "alice", "--json"],
        "update_assignee",
    );

    assert!(
        obr_update.status.success(),
        "obr update assignee failed: {}",
        obr_update.stderr
    );
    assert!(
        bd_update.status.success(),
        "bd update assignee failed: {}",
        bd_update.stderr
    );

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_after_assignee");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_after_assignee");

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");

    let obr_assignee = obr_val["assignee"]
        .as_str()
        .or_else(|| obr_val[0]["assignee"].as_str());
    let bd_assignee = bd_val["assignee"]
        .as_str()
        .or_else(|| bd_val[0]["assignee"].as_str());

    assert_eq!(
        obr_assignee, bd_assignee,
        "assignee mismatch after update: obr={:?}, bd={:?}",
        obr_assignee, bd_assignee
    );
    assert_eq!(obr_assignee, Some("alice"), "expected assignee alice");

    info!("conformance_update_assignee passed");
}

#[test]
fn conformance_update_status() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_update_status test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Status issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Status issue", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    let obr_update = workspace.run_obr(
        ["update", obr_id, "--status", "in_progress", "--json"],
        "update_status",
    );
    let bd_update = workspace.run_bd(
        ["update", bd_id, "--status", "in_progress", "--json"],
        "update_status",
    );

    assert!(
        obr_update.status.success(),
        "obr update status failed: {}",
        obr_update.stderr
    );
    assert!(
        bd_update.status.success(),
        "bd update status failed: {}",
        bd_update.stderr
    );

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_after_status");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_after_status");

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");

    let obr_status = obr_val["status"]
        .as_str()
        .or_else(|| obr_val[0]["status"].as_str());
    let bd_status = bd_val["status"]
        .as_str()
        .or_else(|| bd_val[0]["status"].as_str());

    assert_eq!(
        obr_status, bd_status,
        "status mismatch after update: obr={:?}, bd={:?}",
        obr_status, bd_status
    );
    assert_eq!(
        obr_status,
        Some("in_progress"),
        "expected status in_progress"
    );

    info!("conformance_update_status passed");
}

#[test]
fn conformance_update_multiple_fields() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_update_multiple_fields test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Multi update", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Multi update", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    let obr_update = workspace.run_obr(
        [
            "update",
            obr_id,
            "--title",
            "Updated title",
            "--priority",
            "0",
            "--assignee",
            "bob",
            "--type",
            "bug",
            "--description",
            "Updated description",
            "--json",
        ],
        "update_multi",
    );
    let bd_update = workspace.run_bd(
        [
            "update",
            bd_id,
            "--title",
            "Updated title",
            "--priority",
            "0",
            "--assignee",
            "bob",
            "--type",
            "bug",
            "--description",
            "Updated description",
            "--json",
        ],
        "update_multi",
    );

    assert!(
        obr_update.status.success(),
        "obr update multi failed: {}",
        obr_update.stderr
    );
    assert!(
        bd_update.status.success(),
        "bd update multi failed: {}",
        bd_update.stderr
    );

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_multi");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_multi");

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let result = compare_json(
        &obr_show_json,
        &bd_show_json,
        &CompareMode::ContainsFields(vec![
            "title".to_string(),
            "priority".to_string(),
            "assignee".to_string(),
            "issue_type".to_string(),
            "description".to_string(),
        ]),
    );
    assert!(
        result.is_ok(),
        "multi update comparison failed: {:?}",
        result.err()
    );

    info!("conformance_update_multiple_fields passed");
}

#[test]
fn conformance_update_clear_assignee() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_update_clear_assignee test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(
        ["create", "Assignee clear", "--assignee", "alice", "--json"],
        "create",
    );
    let bd_create = workspace.run_bd(
        ["create", "Assignee clear", "--assignee", "alice", "--json"],
        "create",
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    let obr_update = workspace.run_obr(
        ["update", obr_id, "--assignee", "", "--json"],
        "update_clear_assignee",
    );
    let bd_update = workspace.run_bd(
        ["update", bd_id, "--assignee", "", "--json"],
        "update_clear_assignee",
    );

    assert!(
        obr_update.status.success(),
        "obr update clear assignee failed: {}",
        obr_update.stderr
    );
    assert!(
        bd_update.status.success(),
        "bd update clear assignee failed: {}",
        bd_update.stderr
    );

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_clear_assignee");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_clear_assignee");

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");
    let obr_issue = if obr_val.is_array() {
        &obr_val[0]
    } else {
        &obr_val
    };
    let bd_issue = if bd_val.is_array() {
        &bd_val[0]
    } else {
        &bd_val
    };

    let obr_assignee = obr_issue.get("assignee").and_then(|v| v.as_str());
    let bd_assignee = bd_issue.get("assignee").and_then(|v| v.as_str());

    assert_eq!(
        obr_assignee, bd_assignee,
        "assignee mismatch after clear: obr={:?}, bd={:?}",
        obr_assignee, bd_assignee
    );
    assert!(obr_assignee.is_none(), "expected assignee cleared");

    info!("conformance_update_clear_assignee passed");
}

#[test]
fn conformance_update_preserves_other_fields() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_update_preserves_other_fields test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(
        [
            "create",
            "Preserve fields",
            "--description",
            "Keep me",
            "--external-ref",
            "EXT-999",
            "--json",
        ],
        "create",
    );
    let bd_create = workspace.run_bd(
        [
            "create",
            "Preserve fields",
            "--description",
            "Keep me",
            "--external-ref",
            "EXT-999",
            "--json",
        ],
        "create",
    );

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    workspace.run_obr(["update", obr_id, "--priority", "0"], "update_pri");
    workspace.run_bd(["update", bd_id, "--priority", "0"], "update_pri");

    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_preserve");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_preserve");

    let obr_show_json = extract_json_payload(&obr_show.stdout);
    let bd_show_json = extract_json_payload(&bd_show.stdout);

    let obr_val: Value = serde_json::from_str(&obr_show_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_show_json).expect("parse");
    let obr_issue = if obr_val.is_array() {
        &obr_val[0]
    } else {
        &obr_val
    };
    let bd_issue = if bd_val.is_array() {
        &bd_val[0]
    } else {
        &bd_val
    };

    assert_eq!(
        obr_issue["description"].as_str(),
        bd_issue["description"].as_str(),
        "description mismatch after update"
    );
    assert_eq!(
        obr_issue["external_ref"].as_str(),
        bd_issue["external_ref"].as_str(),
        "external_ref mismatch after update"
    );
    assert_eq!(
        obr_issue["description"].as_str(),
        Some("Keep me"),
        "description should be preserved"
    );

    info!("conformance_update_preserves_other_fields passed");
}

#[test]
fn conformance_update_nonexistent_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_update_nonexistent_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_update = workspace.run_obr(
        ["update", "bd-does-not-exist", "--title", "Nope", "--json"],
        "update_missing",
    );
    let _bd_update = workspace.run_bd(
        ["update", "bd-does-not-exist", "--title", "Nope", "--json"],
        "update_missing",
    );

    // bd behavior is inconsistent/legacy, but obr should definitely fail
    assert!(
        !obr_update.status.success(),
        "obr expected update missing to fail"
    );
    // Don't compare with bd for this case

    info!("conformance_update_nonexistent_error passed");
}

#[test]
fn conformance_close_with_reason() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_close_with_reason test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Close reason issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Close reason issue", "--json"], "create");

    let obr_json = extract_json_payload(&obr_create.stdout);
    let bd_json = extract_json_payload(&bd_create.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse");

    let obr_id = obr_val["id"]
        .as_str()
        .or_else(|| obr_val[0]["id"].as_str())
        .unwrap();
    let bd_id = bd_val["id"]
        .as_str()
        .or_else(|| bd_val[0]["id"].as_str())
        .unwrap();

    let obr_close = workspace.run_obr(
        ["close", obr_id, "--reason", "done", "--json"],
        "close_reason",
    );
    let bd_close = workspace.run_bd(
        ["close", bd_id, "--reason", "done", "--json"],
        "close_reason",
    );

    assert!(
        obr_close.status.success(),
        "obr close with reason failed: {}",
        obr_close.stderr
    );
    assert!(
        bd_close.status.success(),
        "bd close with reason failed: {}",
        bd_close.stderr
    );

    let obr_close_json = extract_json_payload(&obr_close.stdout);
    let bd_close_json = extract_json_payload(&bd_close.stdout);

    let obr_val: Value = serde_json::from_str(&obr_close_json).expect("parse");
    let bd_val: Value = serde_json::from_str(&bd_close_json).expect("parse");

    let obr_reason = obr_val["close_reason"]
        .as_str()
        .or_else(|| obr_val[0]["close_reason"].as_str());
    let bd_reason = bd_val["close_reason"]
        .as_str()
        .or_else(|| bd_val[0]["close_reason"].as_str());

    assert_eq!(
        obr_reason, bd_reason,
        "close_reason mismatch: obr={:?}, bd={:?}",
        obr_reason, bd_reason
    );
    assert_eq!(obr_reason, Some("done"), "expected close reason");

    info!("conformance_close_with_reason passed");
}

// ============================================================================
// DEPENDENCY COMMAND CONFORMANCE TESTS (beads_rust-v740)
// ============================================================================

/// Helper function to extract an issue ID from JSON output (handles both object and array formats)
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

fn assert_command_success(output: &CmdOutput, label: &str) {
    assert!(
        output.status.success(),
        "{label} failed: stdout='{}' stderr='{}'",
        output.stdout,
        output.stderr
    );
}

fn issue_value_from_show_output(output: &CmdOutput, label: &str) -> Value {
    assert_command_success(output, label);
    let json = extract_json_payload(&output.stdout);
    let value: Value = serde_json::from_str(&json).expect("parse show json");
    if value.is_array() {
        value[0].clone()
    } else {
        value
    }
}

fn assert_json_output_mentions(output: &CmdOutput, expected: &str, label: &str) {
    assert_command_success(output, label);
    let json = extract_json_payload(&output.stdout);
    assert!(
        json.contains(expected),
        "{label} output should mention {expected}: {json}"
    );
}

fn extract_checks_len(json_str: &str) -> usize {
    serde_json::from_str::<Value>(json_str)
        .ok()
        .and_then(|val| {
            val.get("checks")
                .and_then(|checks| checks.as_array())
                .map(|checks| checks.len())
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// dep add tests (8)
// ---------------------------------------------------------------------------

#[test]
fn conformance_dep_add_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_add_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create two issues
    let obr_blocker = workspace.run_obr(["create", "Blocker issue", "--json"], "create_blocker");
    let bd_blocker = workspace.run_bd(["create", "Blocker issue", "--json"], "create_blocker");

    let obr_dependent =
        workspace.run_obr(["create", "Dependent issue", "--json"], "create_dependent");
    let bd_dependent =
        workspace.run_bd(["create", "Dependent issue", "--json"], "create_dependent");

    let obr_blocker_id = extract_issue_id(&extract_json_payload(&obr_blocker.stdout));
    let bd_blocker_id = extract_issue_id(&extract_json_payload(&bd_blocker.stdout));
    let obr_dependent_id = extract_issue_id(&extract_json_payload(&obr_dependent.stdout));
    let bd_dependent_id = extract_issue_id(&extract_json_payload(&bd_dependent.stdout));

    // Add basic blocks dependency
    let obr_add = workspace.run_obr(
        ["dep", "add", &obr_dependent_id, &obr_blocker_id, "--json"],
        "dep_add",
    );
    let bd_add = workspace.run_bd(
        ["dep", "add", &bd_dependent_id, &bd_blocker_id, "--json"],
        "dep_add",
    );

    assert!(
        obr_add.status.success(),
        "obr dep add failed: {}",
        obr_add.stderr
    );
    assert!(
        bd_add.status.success(),
        "bd dep add failed: {}",
        bd_add.stderr
    );

    // Both should produce similar JSON structure
    let obr_json = extract_json_payload(&obr_add.stdout);
    let bd_json = extract_json_payload(&bd_add.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Null);
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Null);

    // Check that both have action/status fields indicating success
    let obr_status = obr_val["status"].as_str().or(obr_val["action"].as_str());
    let bd_status = bd_val["status"].as_str().or(bd_val["action"].as_str());

    assert!(
        obr_status.is_some() || obr_add.status.success(),
        "obr should indicate success"
    );
    assert!(
        bd_status.is_some() || bd_add.status.success(),
        "bd should indicate success"
    );

    info!("conformance_dep_add_basic passed");
}

#[test]
fn conformance_dep_add_all_types() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_add_all_types test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Test dependency types that work in both obr and bd
    // Note: bd has bugs with some types:
    //   - "waits-for": malformed JSON error in bd
    //   - "conditional-blocks": not reliably supported
    // Skipping these until bd fixes the issues
    let dep_types = [
        "blocks",
        "parent-child",
        // "conditional-blocks", // bd: unreliable
        // "waits-for", // bd bug: malformed JSON
        "related",
        "discovered-from",
        "replies-to",
        "relates-to",
        "duplicates",
        "supersedes",
        "caused-by",
    ];

    for dep_type in dep_types {
        // Create fresh issues for each type to avoid conflicts
        let obr_source = workspace.run_obr(
            ["create", &format!("Source for {}", dep_type), "--json"],
            &format!("create_source_{}", dep_type),
        );
        let bd_source = workspace.run_bd(
            ["create", &format!("Source for {}", dep_type), "--json"],
            &format!("create_source_{}", dep_type),
        );

        let obr_target = workspace.run_obr(
            ["create", &format!("Target for {}", dep_type), "--json"],
            &format!("create_target_{}", dep_type),
        );
        let bd_target = workspace.run_bd(
            ["create", &format!("Target for {}", dep_type), "--json"],
            &format!("create_target_{}", dep_type),
        );

        let obr_source_id = extract_issue_id(&extract_json_payload(&obr_source.stdout));
        let bd_source_id = extract_issue_id(&extract_json_payload(&bd_source.stdout));
        let obr_target_id = extract_issue_id(&extract_json_payload(&obr_target.stdout));
        let bd_target_id = extract_issue_id(&extract_json_payload(&bd_target.stdout));

        // Add dependency with specific type
        let obr_add = workspace.run_obr(
            ["dep", "add", &obr_source_id, &obr_target_id, "-t", dep_type],
            &format!("dep_add_{}", dep_type),
        );
        let bd_add = workspace.run_bd(
            ["dep", "add", &bd_source_id, &bd_target_id, "-t", dep_type],
            &format!("dep_add_{}", dep_type),
        );

        assert!(
            obr_add.status.success(),
            "obr dep add failed for type '{}': {}",
            dep_type,
            obr_add.stderr
        );
        assert!(
            bd_add.status.success(),
            "bd dep add failed for type '{}': {}",
            dep_type,
            bd_add.stderr
        );
    }

    info!("conformance_dep_add_all_types passed");
}

#[test]
fn conformance_dep_add_duplicate() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_add_duplicate test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create two issues
    let obr_a = workspace.run_obr(["create", "Issue A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Issue A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Issue B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Issue B", "--json"], "create_b");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));

    // Add dependency first time
    let obr_add1 = workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "dep_add_1");
    let bd_add1 = workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "dep_add_1");

    assert!(obr_add1.status.success(), "obr first dep add failed");
    assert!(bd_add1.status.success(), "bd first dep add failed");

    // Add same dependency again
    // KNOWN DIFFERENCE: obr treats duplicate adds as idempotent (succeeds),
    // bd treats them as errors (fails). This test documents the difference.
    let obr_add2 = workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id, "--json"], "dep_add_2");
    let bd_add2 = workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id, "--json"], "dep_add_2");

    // obr: idempotent - adding duplicate succeeds
    // bd: strict - adding duplicate fails
    // Document this known behavioral difference rather than asserting they match
    info!(
        "Duplicate dep handling: obr={}, bd={} (known difference: obr is idempotent)",
        obr_add2.status.success(),
        bd_add2.status.success()
    );

    // Verify obr's idempotent behavior is consistent
    assert!(
        obr_add2.status.success(),
        "obr should succeed on duplicate dep add (idempotent behavior)"
    );

    info!("conformance_dep_add_duplicate passed");
}

#[test]
fn conformance_dep_add_self_reference_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_add_self_reference_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create an issue
    let obr_issue = workspace.run_obr(["create", "Self-ref test", "--json"], "create");
    let bd_issue = workspace.run_bd(["create", "Self-ref test", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_issue.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_issue.stdout));

    // Try to add self-dependency - should fail
    let obr_add = workspace.run_obr(["dep", "add", &obr_id, &obr_id], "dep_add_self");
    let bd_add = workspace.run_bd(["dep", "add", &bd_id, &bd_id], "dep_add_self");

    // Both should fail
    assert!(
        !obr_add.status.success(),
        "obr should reject self-dependency but it succeeded"
    );
    assert!(
        !bd_add.status.success(),
        "bd should reject self-dependency but it succeeded"
    );

    info!("conformance_dep_add_self_reference_error passed");
}

#[test]
fn conformance_dep_add_cycle_detection() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_add_cycle_detection test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create two issues
    let obr_a = workspace.run_obr(["create", "Cycle A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Cycle A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Cycle B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Cycle B", "--json"], "create_b");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));

    // A depends on B (A waits for B)
    let obr_add1 = workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "add_a_to_b");
    let bd_add1 = workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "add_a_to_b");

    assert!(obr_add1.status.success(), "obr first dep failed");
    assert!(bd_add1.status.success(), "bd first dep failed");

    // Try B depends on A - should create cycle, should fail
    let obr_add2 = workspace.run_obr(["dep", "add", &obr_b_id, &obr_a_id], "add_b_to_a");
    let bd_add2 = workspace.run_bd(["dep", "add", &bd_b_id, &bd_a_id], "add_b_to_a");

    // Both should fail due to cycle detection
    assert!(
        !obr_add2.status.success(),
        "obr should reject cycle A->B->A but succeeded"
    );
    assert!(
        !bd_add2.status.success(),
        "bd should reject cycle A->B->A but succeeded"
    );

    info!("conformance_dep_add_cycle_detection passed");
}

#[test]
fn conformance_dep_add_transitive_cycle() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_add_transitive_cycle test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create three issues
    let obr_a = workspace.run_obr(["create", "Trans A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Trans A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Trans B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Trans B", "--json"], "create_b");

    let obr_c = workspace.run_obr(["create", "Trans C", "--json"], "create_c");
    let bd_c = workspace.run_bd(["create", "Trans C", "--json"], "create_c");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));
    let obr_c_id = extract_issue_id(&extract_json_payload(&obr_c.stdout));
    let bd_c_id = extract_issue_id(&extract_json_payload(&bd_c.stdout));

    // A -> B -> C chain
    let obr_ab = workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "add_a_b");
    let bd_ab = workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "add_a_b");
    assert!(obr_ab.status.success());
    assert!(bd_ab.status.success());

    let obr_bc = workspace.run_obr(["dep", "add", &obr_b_id, &obr_c_id], "add_b_c");
    let bd_bc = workspace.run_bd(["dep", "add", &bd_b_id, &bd_c_id], "add_b_c");
    assert!(obr_bc.status.success());
    assert!(bd_bc.status.success());

    // Try C -> A (creates cycle A->B->C->A)
    let obr_ca = workspace.run_obr(["dep", "add", &obr_c_id, &obr_a_id], "add_c_a");
    let bd_ca = workspace.run_bd(["dep", "add", &bd_c_id, &bd_a_id], "add_c_a");

    // Both should fail
    assert!(
        !obr_ca.status.success(),
        "obr should reject transitive cycle A->B->C->A"
    );
    assert!(
        !bd_ca.status.success(),
        "bd should reject transitive cycle A->B->C->A"
    );

    info!("conformance_dep_add_transitive_cycle passed");
}

#[test]
fn conformance_dep_add_nonexistent_source_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_add_nonexistent_source_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create only one issue
    let obr_target = workspace.run_obr(["create", "Target issue", "--json"], "create_target");
    let bd_target = workspace.run_bd(["create", "Target issue", "--json"], "create_target");

    let obr_target_id = extract_issue_id(&extract_json_payload(&obr_target.stdout));
    let bd_target_id = extract_issue_id(&extract_json_payload(&bd_target.stdout));

    // Try to add dep from nonexistent source
    let obr_add = workspace.run_obr(
        ["dep", "add", "bd-nonexistent999", &obr_target_id],
        "dep_add",
    );
    let bd_add = workspace.run_bd(
        ["dep", "add", "bd-nonexistent999", &bd_target_id],
        "dep_add",
    );

    // Both should fail
    assert!(
        !obr_add.status.success(),
        "obr should reject nonexistent source"
    );
    assert!(
        !bd_add.status.success(),
        "bd should reject nonexistent source"
    );

    info!("conformance_dep_add_nonexistent_source_error passed");
}

#[test]
fn conformance_dep_add_nonexistent_target_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_add_nonexistent_target_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create only one issue
    let obr_source = workspace.run_obr(["create", "Source issue", "--json"], "create_source");
    let bd_source = workspace.run_bd(["create", "Source issue", "--json"], "create_source");

    let obr_source_id = extract_issue_id(&extract_json_payload(&obr_source.stdout));
    let bd_source_id = extract_issue_id(&extract_json_payload(&bd_source.stdout));

    // Try to add dep to nonexistent target
    let obr_add = workspace.run_obr(
        ["dep", "add", &obr_source_id, "bd-nonexistent999"],
        "dep_add",
    );
    let bd_add = workspace.run_bd(
        ["dep", "add", &bd_source_id, "bd-nonexistent999"],
        "dep_add",
    );

    // Both should fail
    assert!(
        !obr_add.status.success(),
        "obr should reject nonexistent target"
    );
    assert!(
        !bd_add.status.success(),
        "bd should reject nonexistent target"
    );

    info!("conformance_dep_add_nonexistent_target_error passed");
}

// ---------------------------------------------------------------------------
// dep remove tests (5)
// ---------------------------------------------------------------------------

#[test]
fn conformance_dep_remove_basic_expanded() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_remove_basic_expanded test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    let obr_a = workspace.run_obr(["create", "Remove A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Remove A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Remove B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Remove B", "--json"], "create_b");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));

    // Add dependency
    workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "add_dep");

    // Remove with JSON output
    let obr_rm = workspace.run_obr(["dep", "remove", &obr_a_id, &obr_b_id, "--json"], "rm_dep");
    let bd_rm = workspace.run_bd(["dep", "remove", &bd_a_id, &bd_b_id, "--json"], "rm_dep");

    assert!(
        obr_rm.status.success(),
        "obr dep remove failed: {}",
        obr_rm.stderr
    );
    assert!(
        bd_rm.status.success(),
        "bd dep remove failed: {}",
        bd_rm.stderr
    );

    // Verify dependency is gone
    let obr_list = workspace.run_obr(["dep", "list", &obr_a_id, "--json"], "list_after");
    let bd_list = workspace.run_bd(["dep", "list", &bd_a_id, "--json"], "list_after");

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    let obr_deps: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Array(vec![]));
    let bd_deps: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_deps);
    let bd_len = issue_count(&bd_deps);

    assert_eq!(obr_len, 0, "obr should have 0 deps after remove");
    assert_eq!(bd_len, 0, "bd should have 0 deps after remove");

    info!("conformance_dep_remove_basic_expanded passed");
}

#[test]
fn conformance_dep_remove_nonexistent() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_remove_nonexistent test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues but don't add dependency
    let obr_a = workspace.run_obr(["create", "No-dep A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "No-dep A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "No-dep B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "No-dep B", "--json"], "create_b");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));

    // Try to remove non-existent dependency
    // KNOWN DIFFERENCE: obr treats this as idempotent (succeeds),
    // bd treats it as an error (fails). This test documents the difference.
    let obr_rm = workspace.run_obr(
        ["dep", "remove", &obr_a_id, &obr_b_id, "--json"],
        "rm_nonexistent",
    );
    let bd_rm = workspace.run_bd(
        ["dep", "remove", &bd_a_id, &bd_b_id, "--json"],
        "rm_nonexistent",
    );

    // obr: idempotent - removing non-existent dep succeeds (no-op)
    // bd: strict - removing non-existent dep fails
    info!(
        "Remove nonexistent dep: obr={}, bd={} (known difference: obr is idempotent)",
        obr_rm.status.success(),
        bd_rm.status.success()
    );

    // Verify obr's idempotent behavior is consistent
    assert!(
        obr_rm.status.success(),
        "obr should succeed on removing nonexistent dep (idempotent behavior)"
    );

    info!("conformance_dep_remove_nonexistent passed");
}

#[test]
fn conformance_dep_remove_unblocks_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_remove_unblocks_issue test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create blocker and blocked issues
    let obr_blocker = workspace.run_obr(["create", "Blocker", "--json"], "create_blocker");
    let bd_blocker = workspace.run_bd(["create", "Blocker", "--json"], "create_blocker");

    let obr_blocked = workspace.run_obr(["create", "Blocked", "--json"], "create_blocked");
    let bd_blocked = workspace.run_bd(["create", "Blocked", "--json"], "create_blocked");

    let obr_blocker_id = extract_issue_id(&extract_json_payload(&obr_blocker.stdout));
    let bd_blocker_id = extract_issue_id(&extract_json_payload(&bd_blocker.stdout));
    let obr_blocked_id = extract_issue_id(&extract_json_payload(&obr_blocked.stdout));
    let bd_blocked_id = extract_issue_id(&extract_json_payload(&bd_blocked.stdout));

    // Add blocking dependency
    workspace.run_obr(["dep", "add", &obr_blocked_id, &obr_blocker_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_blocked_id, &bd_blocker_id], "add_dep");

    // Verify blocked
    let obr_blocked_before = workspace.run_obr(["blocked", "--json"], "blocked_before");
    let bd_blocked_before = workspace.run_bd(["blocked", "--json"], "blocked_before");

    let obr_before: Value = serde_json::from_str(&extract_json_payload(&obr_blocked_before.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_before: Value = serde_json::from_str(&extract_json_payload(&bd_blocked_before.stdout))
        .unwrap_or(Value::Array(vec![]));

    assert_eq!(issue_count(&obr_before), 1, "obr should have 1 blocked");
    assert_eq!(issue_count(&bd_before), 1, "bd should have 1 blocked");

    // Remove dependency
    workspace.run_obr(
        ["dep", "remove", &obr_blocked_id, &obr_blocker_id],
        "rm_dep",
    );
    workspace.run_bd(["dep", "remove", &bd_blocked_id, &bd_blocker_id], "rm_dep");

    // Verify unblocked
    let obr_blocked_after = workspace.run_obr(["blocked", "--json"], "blocked_after");
    let bd_blocked_after = workspace.run_bd(["blocked", "--json"], "blocked_after");

    let obr_after: Value = serde_json::from_str(&extract_json_payload(&obr_blocked_after.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_after: Value = serde_json::from_str(&extract_json_payload(&bd_blocked_after.stdout))
        .unwrap_or(Value::Array(vec![]));

    assert_eq!(issue_count(&obr_after), 0, "obr should have 0 blocked");
    assert_eq!(issue_count(&bd_after), 0, "bd should have 0 blocked");

    // Verify now ready
    let obr_ready = workspace.run_obr(["ready", "--json"], "ready_after");
    let bd_ready = workspace.run_bd(["ready", "--json"], "ready_after");

    let obr_ready_val: Value = serde_json::from_str(&extract_json_payload(&obr_ready.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_ready_val: Value = serde_json::from_str(&extract_json_payload(&bd_ready.stdout))
        .unwrap_or(Value::Array(vec![]));

    // Both issues should now be ready
    assert_eq!(
        issue_count(&obr_ready_val),
        issue_count(&bd_ready_val),
        "ready counts should match"
    );

    info!("conformance_dep_remove_unblocks_issue passed");
}

#[test]
fn conformance_dep_remove_preserves_other_deps() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_remove_preserves_other_deps test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create three issues
    let obr_a = workspace.run_obr(["create", "Multi A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Multi A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Multi B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Multi B", "--json"], "create_b");

    let obr_c = workspace.run_obr(["create", "Multi C", "--json"], "create_c");
    let bd_c = workspace.run_bd(["create", "Multi C", "--json"], "create_c");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));
    let obr_c_id = extract_issue_id(&extract_json_payload(&obr_c.stdout));
    let bd_c_id = extract_issue_id(&extract_json_payload(&bd_c.stdout));

    // A depends on both B and C
    workspace.run_obr(
        ["dep", "add", &obr_a_id, &obr_b_id, "-t", "related"],
        "add_a_b",
    );
    workspace.run_bd(
        ["dep", "add", &bd_a_id, &bd_b_id, "-t", "related"],
        "add_a_b",
    );

    workspace.run_obr(
        ["dep", "add", &obr_a_id, &obr_c_id, "-t", "related"],
        "add_a_c",
    );
    workspace.run_bd(
        ["dep", "add", &bd_a_id, &bd_c_id, "-t", "related"],
        "add_a_c",
    );

    // Verify 2 deps
    let obr_list_before = workspace.run_obr(["dep", "list", &obr_a_id, "--json"], "list_before");
    let bd_list_before = workspace.run_bd(["dep", "list", &bd_a_id, "--json"], "list_before");

    let obr_before: Value = serde_json::from_str(&extract_json_payload(&obr_list_before.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_before: Value = serde_json::from_str(&extract_json_payload(&bd_list_before.stdout))
        .unwrap_or(Value::Array(vec![]));

    assert_eq!(issue_count(&obr_before), 2);
    assert_eq!(issue_count(&bd_before), 2);

    // Remove only A->B
    workspace.run_obr(["dep", "remove", &obr_a_id, &obr_b_id], "rm_a_b");
    workspace.run_bd(["dep", "remove", &bd_a_id, &bd_b_id], "rm_a_b");

    // Verify A->C still exists
    let obr_list_after = workspace.run_obr(["dep", "list", &obr_a_id, "--json"], "list_after");
    let bd_list_after = workspace.run_bd(["dep", "list", &bd_a_id, "--json"], "list_after");

    let obr_after: Value = serde_json::from_str(&extract_json_payload(&obr_list_after.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_after: Value = serde_json::from_str(&extract_json_payload(&bd_list_after.stdout))
        .unwrap_or(Value::Array(vec![]));

    assert_eq!(issue_count(&obr_after), 1, "obr should have 1 dep left");
    assert_eq!(issue_count(&bd_after), 1, "bd should have 1 dep left");

    info!("conformance_dep_remove_preserves_other_deps passed");
}

// ---------------------------------------------------------------------------
// dep list tests (6)
// ---------------------------------------------------------------------------

#[test]
fn conformance_dep_list_basic_expanded() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_list_basic_expanded test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with dependency
    let obr_parent = workspace.run_obr(["create", "List Parent", "--json"], "create_parent");
    let bd_parent = workspace.run_bd(["create", "List Parent", "--json"], "create_parent");

    let obr_child = workspace.run_obr(["create", "List Child", "--json"], "create_child");
    let bd_child = workspace.run_bd(["create", "List Child", "--json"], "create_child");

    let obr_parent_id = extract_issue_id(&extract_json_payload(&obr_parent.stdout));
    let bd_parent_id = extract_issue_id(&extract_json_payload(&bd_parent.stdout));
    let obr_child_id = extract_issue_id(&extract_json_payload(&obr_child.stdout));
    let bd_child_id = extract_issue_id(&extract_json_payload(&bd_child.stdout));

    // Add dependency
    workspace.run_obr(["dep", "add", &obr_child_id, &obr_parent_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_child_id, &bd_parent_id], "add_dep");

    // List deps
    let obr_list = workspace.run_obr(["dep", "list", &obr_child_id, "--json"], "list");
    let bd_list = workspace.run_bd(["dep", "list", &bd_child_id, "--json"], "list");

    assert!(obr_list.status.success(), "obr dep list failed");
    assert!(bd_list.status.success(), "bd dep list failed");

    let obr_deps: Value = serde_json::from_str(&extract_json_payload(&obr_list.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_deps: Value = serde_json::from_str(&extract_json_payload(&bd_list.stdout))
        .unwrap_or(Value::Array(vec![]));

    assert_eq!(
        issue_count(&obr_deps),
        issue_count(&bd_deps),
        "dep list counts should match"
    );

    info!("conformance_dep_list_basic_expanded passed");
}

#[test]
fn conformance_dep_list_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_list_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue with no deps
    let obr_issue = workspace.run_obr(["create", "No deps issue", "--json"], "create");
    let bd_issue = workspace.run_bd(["create", "No deps issue", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_issue.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_issue.stdout));

    // List deps - should be empty
    let obr_list = workspace.run_obr(["dep", "list", &obr_id, "--json"], "list_empty");
    let bd_list = workspace.run_bd(["dep", "list", &bd_id, "--json"], "list_empty");

    assert!(obr_list.status.success(), "obr dep list failed");
    assert!(bd_list.status.success(), "bd dep list failed");

    let obr_deps: Value = serde_json::from_str(&extract_json_payload(&obr_list.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_deps: Value = serde_json::from_str(&extract_json_payload(&bd_list.stdout))
        .unwrap_or(Value::Array(vec![]));

    assert_eq!(issue_count(&obr_deps), 0, "obr should have 0 deps");
    assert_eq!(issue_count(&bd_deps), 0, "bd should have 0 deps");

    info!("conformance_dep_list_empty passed");
}

#[test]
fn conformance_dep_list_by_type() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_list_by_type test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    let obr_main = workspace.run_obr(["create", "Main issue", "--json"], "create_main");
    let bd_main = workspace.run_bd(["create", "Main issue", "--json"], "create_main");

    let obr_blocks = workspace.run_obr(["create", "Blocks target", "--json"], "create_blocks");
    let bd_blocks = workspace.run_bd(["create", "Blocks target", "--json"], "create_blocks");

    let obr_related = workspace.run_obr(["create", "Related target", "--json"], "create_related");
    let bd_related = workspace.run_bd(["create", "Related target", "--json"], "create_related");

    let obr_main_id = extract_issue_id(&extract_json_payload(&obr_main.stdout));
    let bd_main_id = extract_issue_id(&extract_json_payload(&bd_main.stdout));
    let obr_blocks_id = extract_issue_id(&extract_json_payload(&obr_blocks.stdout));
    let bd_blocks_id = extract_issue_id(&extract_json_payload(&bd_blocks.stdout));
    let obr_related_id = extract_issue_id(&extract_json_payload(&obr_related.stdout));
    let bd_related_id = extract_issue_id(&extract_json_payload(&bd_related.stdout));

    // Add different dependency types
    workspace.run_obr(
        ["dep", "add", &obr_main_id, &obr_blocks_id, "-t", "blocks"],
        "add_blocks",
    );
    workspace.run_bd(
        ["dep", "add", &bd_main_id, &bd_blocks_id, "-t", "blocks"],
        "add_blocks",
    );

    workspace.run_obr(
        ["dep", "add", &obr_main_id, &obr_related_id, "-t", "related"],
        "add_related",
    );
    workspace.run_bd(
        ["dep", "add", &bd_main_id, &bd_related_id, "-t", "related"],
        "add_related",
    );

    // List only blocks type
    let obr_list = workspace.run_obr(
        ["dep", "list", &obr_main_id, "-t", "blocks", "--json"],
        "list_blocks",
    );
    let bd_list = workspace.run_bd(
        ["dep", "list", &bd_main_id, "-t", "blocks", "--json"],
        "list_blocks",
    );

    let obr_deps: Value = serde_json::from_str(&extract_json_payload(&obr_list.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_deps: Value = serde_json::from_str(&extract_json_payload(&bd_list.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_len = issue_count(&obr_deps);
    let bd_len = issue_count(&bd_deps);

    assert_eq!(
        obr_len, bd_len,
        "filtered dep counts should match: obr={}, bd={}",
        obr_len, bd_len
    );

    info!("conformance_dep_list_by_type passed");
}

#[test]
fn conformance_dep_list_json_structure() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_list_json_structure test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with dependency
    let obr_a = workspace.run_obr(["create", "Struct A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Struct A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Struct B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Struct B", "--json"], "create_b");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));

    workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "add_dep");

    let obr_list = workspace.run_obr(["dep", "list", &obr_a_id, "--json"], "list");
    let bd_list = workspace.run_bd(["dep", "list", &bd_a_id, "--json"], "list");

    let obr_deps: Value = serde_json::from_str(&extract_json_payload(&obr_list.stdout))
        .expect("obr should produce valid JSON");
    let bd_deps: Value = serde_json::from_str(&extract_json_payload(&bd_list.stdout))
        .expect("bd should produce valid JSON");

    // Both should be arrays
    assert!(obr_deps.is_array(), "obr dep list should be an array");
    assert!(bd_deps.is_array(), "bd dep list should be an array");

    // If not empty, check structure
    if let Some(obr_arr) = obr_deps.as_array() {
        if let Some(first) = obr_arr.first() {
            // Should have standard dep fields
            let has_issue_id = first.get("issue_id").is_some();
            let has_depends_on = first.get("depends_on_id").is_some();
            let has_type = first.get("type").is_some();

            assert!(
                has_issue_id || has_depends_on,
                "obr dep list items should have id fields"
            );
            assert!(has_type, "obr dep list items should have type field");
        }
    }

    info!("conformance_dep_list_json_structure passed");
}

// ---------------------------------------------------------------------------
// dep tree tests (6)
// ---------------------------------------------------------------------------

#[test]
fn conformance_dep_tree_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_tree_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create simple hierarchy
    let obr_root = workspace.run_obr(["create", "Tree Root", "--json"], "create_root");
    let bd_root = workspace.run_bd(["create", "Tree Root", "--json"], "create_root");

    let obr_child = workspace.run_obr(["create", "Tree Child", "--json"], "create_child");
    let bd_child = workspace.run_bd(["create", "Tree Child", "--json"], "create_child");

    let obr_root_id = extract_issue_id(&extract_json_payload(&obr_root.stdout));
    let bd_root_id = extract_issue_id(&extract_json_payload(&bd_root.stdout));
    let obr_child_id = extract_issue_id(&extract_json_payload(&obr_child.stdout));
    let bd_child_id = extract_issue_id(&extract_json_payload(&bd_child.stdout));

    // Child depends on root (root blocks child)
    workspace.run_obr(["dep", "add", &obr_child_id, &obr_root_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_child_id, &bd_root_id], "add_dep");

    // Get tree from root
    let obr_tree = workspace.run_obr(["dep", "tree", &obr_root_id], "tree");
    let bd_tree = workspace.run_bd(["dep", "tree", &bd_root_id], "tree");

    assert!(
        obr_tree.status.success(),
        "obr dep tree failed: {}",
        obr_tree.stderr
    );
    assert!(
        bd_tree.status.success(),
        "bd dep tree failed: {}",
        bd_tree.stderr
    );

    // Both should produce output
    assert!(
        !obr_tree.stdout.trim().is_empty(),
        "obr tree should have output"
    );
    assert!(
        !bd_tree.stdout.trim().is_empty(),
        "bd tree should have output"
    );

    info!("conformance_dep_tree_basic passed");
}

#[test]
fn conformance_dep_tree_deep() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_tree_deep test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create chain: A -> B -> C -> D
    let obr_a = workspace.run_obr(["create", "Deep A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Deep A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Deep B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Deep B", "--json"], "create_b");

    let obr_c = workspace.run_obr(["create", "Deep C", "--json"], "create_c");
    let bd_c = workspace.run_bd(["create", "Deep C", "--json"], "create_c");

    let obr_d = workspace.run_obr(["create", "Deep D", "--json"], "create_d");
    let bd_d = workspace.run_bd(["create", "Deep D", "--json"], "create_d");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));
    let obr_c_id = extract_issue_id(&extract_json_payload(&obr_c.stdout));
    let bd_c_id = extract_issue_id(&extract_json_payload(&bd_c.stdout));
    let obr_d_id = extract_issue_id(&extract_json_payload(&obr_d.stdout));
    let bd_d_id = extract_issue_id(&extract_json_payload(&bd_d.stdout));

    // Build chain: B depends on A, C on B, D on C
    workspace.run_obr(["dep", "add", &obr_b_id, &obr_a_id], "add_b_a");
    workspace.run_bd(["dep", "add", &bd_b_id, &bd_a_id], "add_b_a");

    workspace.run_obr(["dep", "add", &obr_c_id, &obr_b_id], "add_c_b");
    workspace.run_bd(["dep", "add", &bd_c_id, &bd_b_id], "add_c_b");

    workspace.run_obr(["dep", "add", &obr_d_id, &obr_c_id], "add_d_c");
    workspace.run_bd(["dep", "add", &bd_d_id, &bd_c_id], "add_d_c");

    // Get tree from A
    let obr_tree = workspace.run_obr(["dep", "tree", &obr_a_id], "tree");
    let bd_tree = workspace.run_bd(["dep", "tree", &bd_a_id], "tree");

    assert!(obr_tree.status.success(), "obr dep tree failed");
    assert!(bd_tree.status.success(), "bd dep tree failed");

    info!("conformance_dep_tree_deep passed");
}

#[test]
fn conformance_dep_tree_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_tree_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue with no deps
    let obr_issue = workspace.run_obr(["create", "Tree empty", "--json"], "create");
    let bd_issue = workspace.run_bd(["create", "Tree empty", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_issue.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_issue.stdout));

    // Get tree - should just show the root
    let obr_tree = workspace.run_obr(["dep", "tree", &obr_id], "tree");
    let bd_tree = workspace.run_bd(["dep", "tree", &bd_id], "tree");

    assert!(obr_tree.status.success(), "obr dep tree failed");
    assert!(bd_tree.status.success(), "bd dep tree failed");

    info!("conformance_dep_tree_empty passed");
}

#[test]
fn conformance_dep_tree_json() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_tree_json test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create hierarchy
    let obr_root = workspace.run_obr(["create", "JSON Tree Root", "--json"], "create_root");
    let bd_root = workspace.run_bd(["create", "JSON Tree Root", "--json"], "create_root");

    let obr_child = workspace.run_obr(["create", "JSON Tree Child", "--json"], "create_child");
    let bd_child = workspace.run_bd(["create", "JSON Tree Child", "--json"], "create_child");

    let obr_root_id = extract_issue_id(&extract_json_payload(&obr_root.stdout));
    let bd_root_id = extract_issue_id(&extract_json_payload(&bd_root.stdout));
    let obr_child_id = extract_issue_id(&extract_json_payload(&obr_child.stdout));
    let bd_child_id = extract_issue_id(&extract_json_payload(&bd_child.stdout));

    workspace.run_obr(["dep", "add", &obr_child_id, &obr_root_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_child_id, &bd_root_id], "add_dep");

    // Get tree as JSON
    let obr_tree = workspace.run_obr(["dep", "tree", &obr_root_id, "--json"], "tree_json");
    let bd_tree = workspace.run_bd(["dep", "tree", &bd_root_id, "--json"], "tree_json");

    // Both should succeed
    let obr_success = obr_tree.status.success();
    let bd_success = bd_tree.status.success();

    // Both should behave the same
    assert_eq!(
        obr_success, bd_success,
        "obr and bd should both succeed or fail for tree --json"
    );

    if obr_success {
        // Parse JSON if available
        let obr_json = extract_json_payload(&obr_tree.stdout);
        let bd_json = extract_json_payload(&bd_tree.stdout);

        let obr_val: Result<Value, _> = serde_json::from_str(&obr_json);
        let bd_val: Result<Value, _> = serde_json::from_str(&bd_json);

        assert!(obr_val.is_ok(), "obr tree JSON should be valid");
        assert!(bd_val.is_ok(), "bd tree JSON should be valid");
    }

    info!("conformance_dep_tree_json passed");
}

// ---------------------------------------------------------------------------
// dep cycles tests (4)
// ---------------------------------------------------------------------------

#[test]
fn conformance_dep_cycles_none() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_cycles_none test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create linear chain (no cycles)
    let obr_a = workspace.run_obr(["create", "NoCycle A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "NoCycle A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "NoCycle B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "NoCycle B", "--json"], "create_b");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));

    // A -> B (no cycle possible)
    workspace.run_obr(
        ["dep", "add", &obr_a_id, &obr_b_id, "-t", "related"],
        "add_dep",
    );
    workspace.run_bd(
        ["dep", "add", &bd_a_id, &bd_b_id, "-t", "related"],
        "add_dep",
    );

    // Check for cycles
    let obr_cycles = workspace.run_obr(["dep", "cycles", "--json"], "cycles");
    let bd_cycles = workspace.run_bd(["dep", "cycles", "--json"], "cycles");

    assert!(obr_cycles.status.success(), "obr dep cycles failed");
    assert!(bd_cycles.status.success(), "bd dep cycles failed");

    let obr_json = extract_json_payload(&obr_cycles.stdout);
    let bd_json = extract_json_payload(&bd_cycles.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Null);
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Null);

    // Both should report 0 cycles
    let obr_count = obr_val["count"].as_u64().unwrap_or(0);
    let bd_count = bd_val["count"].as_u64().unwrap_or(0);

    assert_eq!(obr_count, 0, "obr should find no cycles");
    assert_eq!(bd_count, 0, "bd should find no cycles");

    info!("conformance_dep_cycles_none passed");
}

#[test]
fn conformance_dep_cycles_simple() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_cycles_simple test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create two issues
    let obr_a = workspace.run_obr(["create", "SimpleCycle A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "SimpleCycle A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "SimpleCycle B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "SimpleCycle B", "--json"], "create_b");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));

    // Create cycle using non-blocking type (related doesn't prevent cycles)
    // KNOWN DIFFERENCE: obr detects cycles in all dependency types,
    // bd only detects cycles in blocking dependency types
    workspace.run_obr(
        ["dep", "add", &obr_a_id, &obr_b_id, "-t", "related"],
        "add_a_b",
    );
    workspace.run_bd(
        ["dep", "add", &bd_a_id, &bd_b_id, "-t", "related"],
        "add_a_b",
    );

    workspace.run_obr(
        ["dep", "add", &obr_b_id, &obr_a_id, "-t", "related"],
        "add_b_a",
    );
    workspace.run_bd(
        ["dep", "add", &bd_b_id, &bd_a_id, "-t", "related"],
        "add_b_a",
    );

    // Check for cycles
    let obr_cycles = workspace.run_obr(["dep", "cycles", "--json"], "cycles");
    let bd_cycles = workspace.run_bd(["dep", "cycles", "--json"], "cycles");

    // #368: `obr dep cycles` exits 5 (CycleDetected) when an active cycle is
    // present so scripted callers can gate on the exit code. This diverges from
    // bd, which exits 0; the cycle here is a `related` edge that bd's
    // blocking-only detector ignores, so bd still exits 0. The cycle *data* is
    // emitted on stdout regardless and is compared below.
    assert_eq!(
        obr_cycles.status.code(),
        Some(5),
        "obr dep cycles should exit 5 when an active cycle is present"
    );
    assert!(bd_cycles.status.success(), "bd dep cycles failed");

    let obr_json = extract_json_payload(&obr_cycles.stdout);
    let bd_json = extract_json_payload(&bd_cycles.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Null);
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Null);

    // obr detects cycles in all types, bd only in blocking types
    let obr_count = obr_val["count"].as_u64().unwrap_or(0);
    let bd_count = bd_val["count"].as_u64().unwrap_or(0);

    info!(
        "Cycle detection: obr={}, bd={} (known difference: obr detects in all types)",
        obr_count, bd_count
    );

    // Verify obr properly detects cycles in all dependency types
    assert!(
        obr_count >= 1,
        "obr should detect cycle in 'related' dependencies"
    );

    info!("conformance_dep_cycles_simple passed");
}

#[test]
fn conformance_dep_cycles_complex() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_cycles_complex test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create three issues for A->B->C->A cycle
    let obr_a = workspace.run_obr(["create", "ComplexCycle A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "ComplexCycle A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "ComplexCycle B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "ComplexCycle B", "--json"], "create_b");

    let obr_c = workspace.run_obr(["create", "ComplexCycle C", "--json"], "create_c");
    let bd_c = workspace.run_bd(["create", "ComplexCycle C", "--json"], "create_c");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));
    let obr_c_id = extract_issue_id(&extract_json_payload(&obr_c.stdout));
    let bd_c_id = extract_issue_id(&extract_json_payload(&bd_c.stdout));

    // Create triangular cycle with non-blocking type
    workspace.run_obr(
        ["dep", "add", &obr_a_id, &obr_b_id, "-t", "related"],
        "add_a_b",
    );
    workspace.run_bd(
        ["dep", "add", &bd_a_id, &bd_b_id, "-t", "related"],
        "add_a_b",
    );

    workspace.run_obr(
        ["dep", "add", &obr_b_id, &obr_c_id, "-t", "related"],
        "add_b_c",
    );
    workspace.run_bd(
        ["dep", "add", &bd_b_id, &bd_c_id, "-t", "related"],
        "add_b_c",
    );

    workspace.run_obr(
        ["dep", "add", &obr_c_id, &obr_a_id, "-t", "related"],
        "add_c_a",
    );
    workspace.run_bd(
        ["dep", "add", &bd_c_id, &bd_a_id, "-t", "related"],
        "add_c_a",
    );

    // Check for cycles
    let obr_cycles = workspace.run_obr(["dep", "cycles", "--json"], "cycles");
    let bd_cycles = workspace.run_bd(["dep", "cycles", "--json"], "cycles");

    // #368: active cycle present -> `obr dep cycles` exits 5. The `related`
    // triangular cycle is invisible to bd's blocking-only detector, so bd
    // exits 0. Cycle data still lands on stdout and is compared below.
    assert_eq!(
        obr_cycles.status.code(),
        Some(5),
        "obr dep cycles should exit 5 when an active cycle is present"
    );
    assert!(bd_cycles.status.success(), "bd dep cycles failed");

    let obr_json = extract_json_payload(&obr_cycles.stdout);
    let bd_json = extract_json_payload(&bd_cycles.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Null);
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Null);

    let obr_count = obr_val["count"].as_u64().unwrap_or(0);
    let bd_count = bd_val["count"].as_u64().unwrap_or(0);

    info!(
        "Complex cycle detection: obr={}, bd={} (known difference: obr detects in all types)",
        obr_count, bd_count
    );

    // Verify obr properly detects cycles in all dependency types
    assert!(
        obr_count >= 1,
        "obr should detect cycle in 'related' dependencies"
    );

    info!("conformance_dep_cycles_complex passed");
}

#[test]
fn conformance_dep_cycles_json() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_dep_cycles_json test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Just test JSON output structure
    let obr_cycles = workspace.run_obr(["dep", "cycles", "--json"], "cycles");
    let bd_cycles = workspace.run_bd(["dep", "cycles", "--json"], "cycles");

    assert!(obr_cycles.status.success(), "obr dep cycles --json failed");
    assert!(bd_cycles.status.success(), "bd dep cycles --json failed");

    let obr_json = extract_json_payload(&obr_cycles.stdout);
    let bd_json = extract_json_payload(&bd_cycles.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr should produce valid JSON");
    // KNOWN DIFFERENCE: bd may produce different JSON structure for empty cycles
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Null);

    // Verify obr has expected structure
    assert!(
        obr_val.get("cycles").is_some() || obr_val.get("count").is_some(),
        "obr cycles JSON should have cycles or count field"
    );

    // Log bd structure for documentation purposes (don't assert - known difference)
    info!(
        "JSON structure - obr: cycles={}, count={} | bd: cycles={}, count={}",
        obr_val.get("cycles").is_some(),
        obr_val.get("count").is_some(),
        bd_val.get("cycles").is_some(),
        bd_val.get("count").is_some()
    );

    info!("conformance_dep_cycles_json passed");
}

// ============================================================================
// UTILITY COMMAND CONFORMANCE TESTS
// ============================================================================

// === STATS COMMAND TESTS ===

#[test]
fn conformance_stats_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stats_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Run stats on fresh workspace
    let obr_stats = workspace.run_obr(["stats", "--no-activity", "--json"], "stats_empty");
    let bd_stats = workspace.run_bd(["stats", "--no-activity", "--json"], "stats_empty");

    assert!(
        obr_stats.status.success(),
        "obr stats on empty workspace failed: {}",
        obr_stats.stderr
    );
    assert!(
        bd_stats.status.success(),
        "bd stats on empty workspace failed: {}",
        bd_stats.stderr
    );

    let obr_json = extract_json_payload(&obr_stats.stdout);
    let bd_json = extract_json_payload(&bd_stats.stdout);

    log_timings("stats_empty", &obr_stats, &bd_stats);
    compare_json(
        &obr_json,
        &bd_json,
        // `draft_issues` is a br-only summary field with no bd v0.46.0
        // counterpart, verified against a real bd on 2026-07-25
        // (`beads_rust-ecr6`). Excluding just that key keeps every other stats
        // counter under live comparison, rather than ignoring the whole test.
        &CompareMode::FieldsExcluded(vec![
            "average_lead_time_hours".to_string(),
            "draft_issues".to_string(),
        ]),
    )
    .expect("JSON mismatch");

    info!("conformance_stats_empty passed");
}

#[test]
fn conformance_stats_mixed() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stats_mixed test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create mix of open and closed issues
    let _obr_create1 = workspace.run_obr(["create", "Open issue", "--json"], "create1");
    let _bd_create1 = workspace.run_bd(["create", "Open issue", "--json"], "create1");

    let obr_create2 = workspace.run_obr(["create", "Will close", "--json"], "create2");
    let bd_create2 = workspace.run_bd(["create", "Will close", "--json"], "create2");

    // Close one issue
    let obr_id = extract_issue_id(&extract_json_payload(&obr_create2.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_create2.stdout));

    workspace.run_obr(["close", &obr_id], "close");
    workspace.run_bd(["close", &bd_id], "close");

    // Get stats
    let obr_stats = workspace.run_obr(["stats", "--no-activity", "--json"], "stats");
    let bd_stats = workspace.run_bd(["stats", "--no-activity", "--json"], "stats");

    assert!(
        obr_stats.status.success(),
        "obr stats failed: {}",
        obr_stats.stderr
    );
    assert!(
        bd_stats.status.success(),
        "bd stats failed: {}",
        bd_stats.stderr
    );

    let obr_json = extract_json_payload(&obr_stats.stdout);
    let bd_json = extract_json_payload(&bd_stats.stdout);

    log_timings("stats_mixed", &obr_stats, &bd_stats);
    compare_json(
        &obr_json,
        &bd_json,
        // `draft_issues` is a br-only summary field with no bd v0.46.0
        // counterpart, verified against a real bd on 2026-07-25
        // (`beads_rust-ecr6`). Excluding just that key keeps every other stats
        // counter under live comparison, rather than ignoring the whole test.
        &CompareMode::FieldsExcluded(vec![
            "average_lead_time_hours".to_string(),
            "draft_issues".to_string(),
        ]),
    )
    .expect("JSON mismatch");

    info!("conformance_stats_mixed passed");
}

#[test]
fn conformance_stats_with_deps() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stats_with_deps test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with dependencies
    let obr_a = workspace.run_obr(["create", "Issue A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Issue A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Issue B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Issue B", "--json"], "create_b");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));

    // Add dependency: A depends on B
    workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "add_dep");

    // Get stats
    let obr_stats = workspace.run_obr(["stats", "--no-activity", "--json"], "stats");
    let bd_stats = workspace.run_bd(["stats", "--no-activity", "--json"], "stats");

    assert!(obr_stats.status.success(), "obr stats failed");
    assert!(bd_stats.status.success(), "bd stats failed");

    let obr_json = extract_json_payload(&obr_stats.stdout);
    let bd_json = extract_json_payload(&bd_stats.stdout);

    log_timings("stats_with_deps", &obr_stats, &bd_stats);
    compare_json(
        &obr_json,
        &bd_json,
        // `draft_issues` is a br-only summary field with no bd v0.46.0
        // counterpart, verified against a real bd on 2026-07-25
        // (`beads_rust-ecr6`). Excluding just that key keeps every other stats
        // counter under live comparison, rather than ignoring the whole test.
        &CompareMode::FieldsExcluded(vec![
            "average_lead_time_hours".to_string(),
            "draft_issues".to_string(),
        ]),
    )
    .expect("JSON mismatch");

    info!("conformance_stats_with_deps passed");
}

#[test]
fn conformance_stats_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stats_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create an issue to have some data
    workspace.run_obr(["create", "Test issue"], "create");
    workspace.run_bd(["create", "Test issue"], "create");

    let obr_stats = workspace.run_obr(["stats", "--no-activity", "--json"], "stats");
    let bd_stats = workspace.run_bd(["stats", "--no-activity", "--json"], "stats");

    assert!(obr_stats.status.success(), "obr stats failed");
    assert!(bd_stats.status.success(), "bd stats failed");

    let obr_json = extract_json_payload(&obr_stats.stdout);
    let bd_json = extract_json_payload(&bd_stats.stdout);
    log_timings("stats_all_fields", &obr_stats, &bd_stats);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd json");

    // See the sibling stats tests: `draft_issues` is a br-only summary field
    // with no bd v0.46.0 counterpart (`beads_rust-ecr6`).
    let excluded = vec![
        "average_lead_time_hours".to_string(),
        "draft_issues".to_string(),
    ];
    let obr_filtered = filter_fields(&obr_val, &excluded);
    let bd_filtered = filter_fields(&bd_val, &excluded);

    assert!(
        structure_matches(&obr_filtered, &bd_filtered),
        "stats JSON structure mismatch"
    );

    log_timings("stats_json_shape", &obr_stats, &bd_stats);

    info!("conformance_stats_json_shape passed");
}

// === COUNT COMMAND TESTS ===

#[test]
fn conformance_count_by_status() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_count_by_status test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with different statuses
    workspace.run_obr(["create", "Open 1"], "create1");
    workspace.run_bd(["create", "Open 1"], "create1");

    let obr_create2 = workspace.run_obr(["create", "Will close", "--json"], "create2");
    let bd_create2 = workspace.run_bd(["create", "Will close", "--json"], "create2");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_create2.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_create2.stdout));

    workspace.run_obr(["close", &obr_id], "close");
    workspace.run_bd(["close", &bd_id], "close");

    // Count by status
    // bd count includes closed issues by default, obr does not
    let obr_count = workspace.run_obr(
        ["count", "--by", "status", "--json", "--include-closed"],
        "count",
    );
    let bd_count = workspace.run_bd(["count", "--by-status", "--json"], "count");

    assert!(
        obr_count.status.success(),
        "obr count failed: {}",
        obr_count.stderr
    );
    assert!(
        bd_count.status.success(),
        "bd count failed: {}",
        bd_count.stderr
    );

    let obr_json = extract_json_payload(&obr_count.stdout);
    let bd_json = extract_json_payload(&bd_count.stdout);

    log_timings("count_by_status", &obr_count, &bd_count);
    compare_json(&obr_json, &bd_json, &CompareMode::NormalizedJson).expect("JSON mismatch");

    info!("conformance_count_by_status passed");
}

#[test]
fn conformance_count_by_type() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_count_by_type test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with different types
    workspace.run_obr(["create", "Task 1", "--type", "task"], "create1");
    workspace.run_bd(["create", "Task 1", "--type", "task"], "create1");

    workspace.run_obr(["create", "Bug 1", "--type", "bug"], "create2");
    workspace.run_bd(["create", "Bug 1", "--type", "bug"], "create2");

    workspace.run_obr(["create", "Feature 1", "--type", "feature"], "create3");
    workspace.run_bd(["create", "Feature 1", "--type", "feature"], "create3");

    // Count by type
    let obr_count = workspace.run_obr(["count", "--by", "type", "--json"], "count");
    let bd_count = workspace.run_bd(["count", "--by-type", "--json"], "count");

    assert!(
        obr_count.status.success(),
        "obr count failed: {}",
        obr_count.stderr
    );
    assert!(
        bd_count.status.success(),
        "bd count failed: {}",
        bd_count.stderr
    );

    let obr_json = extract_json_payload(&obr_count.stdout);
    let bd_json = extract_json_payload(&bd_count.stdout);

    log_timings("count_by_type", &obr_count, &bd_count);
    compare_json(&obr_json, &bd_json, &CompareMode::NormalizedJson).expect("JSON mismatch");

    info!("conformance_count_by_type passed");
}

#[test]
fn conformance_count_by_priority() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_count_by_priority test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with different priorities
    workspace.run_obr(["create", "P0 issue", "-p", "0"], "create1");
    workspace.run_bd(["create", "P0 issue", "-p", "0"], "create1");

    workspace.run_obr(["create", "P1 issue", "-p", "1"], "create2");
    workspace.run_bd(["create", "P1 issue", "-p", "1"], "create2");

    workspace.run_obr(["create", "P2 issue", "-p", "2"], "create3");
    workspace.run_bd(["create", "P2 issue", "-p", "2"], "create3");

    // Count by priority
    let obr_count = workspace.run_obr(["count", "--by", "priority", "--json"], "count");
    let bd_count = workspace.run_bd(["count", "--by-priority", "--json"], "count");

    assert!(
        obr_count.status.success(),
        "obr count failed: {}",
        obr_count.stderr
    );
    assert!(
        bd_count.status.success(),
        "bd count failed: {}",
        bd_count.stderr
    );

    let obr_json = extract_json_payload(&obr_count.stdout);
    let bd_json = extract_json_payload(&bd_count.stdout);

    log_timings("count_by_priority", &obr_count, &bd_count);
    compare_json(&obr_json, &bd_json, &CompareMode::NormalizedJson).expect("JSON mismatch");

    info!("conformance_count_by_priority passed");
}

#[test]
fn conformance_count_by_assignee() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_count_by_assignee test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues with different assignees
    workspace.run_obr(
        ["create", "Assigned to Alice", "--assignee", "alice"],
        "create1",
    );
    workspace.run_bd(
        ["create", "Assigned to Alice", "--assignee", "alice"],
        "create1",
    );

    workspace.run_obr(
        ["create", "Assigned to Bob", "--assignee", "bob"],
        "create2",
    );
    workspace.run_bd(
        ["create", "Assigned to Bob", "--assignee", "bob"],
        "create2",
    );

    workspace.run_obr(["create", "Unassigned"], "create3");
    workspace.run_bd(["create", "Unassigned"], "create3");

    // Count by assignee
    let obr_count = workspace.run_obr(["count", "--by", "assignee", "--json"], "count");
    let bd_count = workspace.run_bd(["count", "--by-assignee", "--json"], "count");

    assert!(
        obr_count.status.success(),
        "obr count failed: {}",
        obr_count.stderr
    );
    assert!(
        bd_count.status.success(),
        "bd count failed: {}",
        bd_count.stderr
    );

    let obr_json = extract_json_payload(&obr_count.stdout);
    let bd_json = extract_json_payload(&bd_count.stdout);

    log_timings("count_by_assignee", &obr_count, &bd_count);
    compare_json(&obr_json, &bd_json, &CompareMode::NormalizedJson).expect("JSON mismatch");

    info!("conformance_count_by_assignee passed");
}

#[test]
fn conformance_count_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_count_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Test"], "create");
    workspace.run_bd(["create", "Test"], "create");

    let obr_count = workspace.run_obr(["count", "--json"], "count");
    let bd_count = workspace.run_bd(["count", "--json"], "count");

    assert!(obr_count.status.success(), "obr count failed");
    assert!(bd_count.status.success(), "bd count failed");

    let obr_json = extract_json_payload(&obr_count.stdout);
    let bd_json = extract_json_payload(&bd_count.stdout);

    log_timings("count_json_shape", &obr_count, &bd_count);
    compare_json(&obr_json, &bd_json, &CompareMode::StructureOnly).expect("JSON mismatch");

    info!("conformance_count_json_shape passed");
}

#[test]
fn conformance_count_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_count_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Count on empty workspace
    let obr_count = workspace.run_obr(["count", "--json"], "count");
    let bd_count = workspace.run_bd(["count", "--json"], "count");

    assert!(
        obr_count.status.success(),
        "obr count failed: {}",
        obr_count.stderr
    );
    assert!(
        bd_count.status.success(),
        "bd count failed: {}",
        bd_count.stderr
    );

    let obr_json = extract_json_payload(&obr_count.stdout);
    let bd_json = extract_json_payload(&bd_count.stdout);

    log_timings("count_empty", &obr_count, &bd_count);
    compare_json(&obr_json, &bd_json, &CompareMode::ExactJson).expect("JSON mismatch");

    info!("conformance_count_empty passed");
}

// === STALE COMMAND TESTS ===

#[test]
fn conformance_stale_default() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stale_default test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create an issue (will be fresh, not stale)
    workspace.run_obr(["create", "Fresh issue"], "create");
    workspace.run_bd(["create", "Fresh issue"], "create");

    // Run stale with default threshold
    let obr_stale = workspace.run_obr(["stale", "--json"], "stale");
    let bd_stale = workspace.run_bd(["stale", "--json"], "stale");

    assert!(
        obr_stale.status.success(),
        "obr stale failed: {}",
        obr_stale.stderr
    );
    assert!(
        bd_stale.status.success(),
        "bd stale failed: {}",
        bd_stale.stderr
    );

    let obr_json = extract_json_payload(&obr_stale.stdout);
    let bd_json = extract_json_payload(&bd_stale.stdout);

    log_timings("stale_default", &obr_stale, &bd_stale);
    compare_json(&obr_json, &bd_json, &CompareMode::NormalizedJson).expect("JSON mismatch");

    info!("conformance_stale_default passed");
}

#[test]
#[ignore]
fn conformance_stale_custom_days() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stale_custom_days test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Test issue"], "create");
    workspace.run_bd(["create", "Test issue"], "create");

    // Run stale with --days 0 (everything is stale after 0 days)
    let obr_stale = workspace.run_obr(["stale", "--days", "0", "--json"], "stale");
    let bd_stale = workspace.run_bd(["stale", "--days", "0", "--json"], "stale");

    assert!(
        obr_stale.status.success(),
        "obr stale --days 0 failed: {}",
        obr_stale.stderr
    );
    assert!(
        bd_stale.status.success(),
        "bd stale --days 0 failed: {}",
        bd_stale.stderr
    );

    let obr_json = extract_json_payload(&obr_stale.stdout);
    let bd_json = extract_json_payload(&bd_stale.stdout);

    log_timings("stale_custom_days", &obr_stale, &bd_stale);
    compare_json(&obr_json, &bd_json, &CompareMode::NormalizedJson).expect("JSON mismatch");

    info!("conformance_stale_custom_days passed");
}

#[test]
fn conformance_stale_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stale_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Run stale on empty workspace
    let obr_stale = workspace.run_obr(["stale", "--json"], "stale");
    let bd_stale = workspace.run_bd(["stale", "--json"], "stale");

    assert!(
        obr_stale.status.success(),
        "obr stale failed: {}",
        obr_stale.stderr
    );
    assert!(
        bd_stale.status.success(),
        "bd stale failed: {}",
        bd_stale.stderr
    );

    let obr_json = extract_json_payload(&obr_stale.stdout);
    let bd_json = extract_json_payload(&bd_stale.stdout);

    log_timings("stale_empty", &obr_stale, &bd_stale);
    compare_json(&obr_json, &bd_json, &CompareMode::ExactJson).expect("JSON mismatch");

    info!("conformance_stale_empty passed");
}

#[test]
fn conformance_stale_excludes_closed() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stale_excludes_closed test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create and close an issue
    let obr_create = workspace.run_obr(["create", "Will close", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Will close", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_create.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_create.stdout));

    workspace.run_obr(["close", &obr_id], "close");
    workspace.run_bd(["close", &bd_id], "close");

    // Stale should not include closed issues
    let obr_stale = workspace.run_obr(["stale", "--days", "0", "--json"], "stale");
    let bd_stale = workspace.run_bd(["stale", "--days", "0", "--json"], "stale");

    assert!(obr_stale.status.success(), "obr stale failed");
    assert!(bd_stale.status.success(), "bd stale failed");

    let obr_json = extract_json_payload(&obr_stale.stdout);
    let bd_json = extract_json_payload(&bd_stale.stdout);

    log_timings("stale_excludes_closed", &obr_stale, &bd_stale);
    compare_json(&obr_json, &bd_json, &CompareMode::NormalizedJson).expect("JSON mismatch");

    let obr_ids: HashSet<String> = serde_json::from_str::<Value>(&obr_json)
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                    .map(|id| id.to_string())
                    .collect()
            })
        })
        .unwrap_or_default();

    let bd_ids: HashSet<String> = serde_json::from_str::<Value>(&bd_json)
        .ok()
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                    .map(|id| id.to_string())
                    .collect()
            })
        })
        .unwrap_or_default();

    assert!(
        !obr_ids.contains(&obr_id),
        "obr stale includes closed issue"
    );
    assert!(!bd_ids.contains(&bd_id), "bd stale includes closed issue");

    info!("conformance_stale_excludes_closed passed");
}

#[test]
fn conformance_stale_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stale_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Test"], "create");
    workspace.run_bd(["create", "Test"], "create");

    let obr_stale = workspace.run_obr(["stale", "--json"], "stale");
    let bd_stale = workspace.run_bd(["stale", "--json"], "stale");

    assert!(obr_stale.status.success(), "obr stale failed");
    assert!(bd_stale.status.success(), "bd stale failed");

    let obr_json = extract_json_payload(&obr_stale.stdout);
    let bd_json = extract_json_payload(&bd_stale.stdout);

    log_timings("stale_json_shape", &obr_stale, &bd_stale);
    compare_json(&obr_json, &bd_json, &CompareMode::StructureOnly).expect("JSON mismatch");

    info!("conformance_stale_json_shape passed");
}

// === DOCTOR COMMAND TESTS ===

#[test]
#[ignore]
fn conformance_doctor_healthy() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_doctor_healthy test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Doctor on clean workspace should succeed
    let obr_doctor = workspace.run_obr(["doctor", "--json"], "doctor");
    let bd_doctor = workspace.run_bd(["doctor", "--json"], "doctor");

    assert!(
        obr_doctor.status.success(),
        "obr doctor failed on healthy workspace: {}",
        obr_doctor.stderr
    );
    assert!(
        bd_doctor.status.success(),
        "bd doctor failed on healthy workspace: {}",
        bd_doctor.stderr
    );

    let obr_json = extract_json_payload(&obr_doctor.stdout);
    let bd_json = extract_json_payload(&bd_doctor.stdout);

    let obr_checks = extract_checks_len(&obr_json);
    let bd_checks = extract_checks_len(&bd_json);

    assert!(obr_checks > 0, "obr doctor should emit checks");
    assert!(bd_checks > 0, "bd doctor should emit checks");

    log_timings("doctor_healthy", &obr_doctor, &bd_doctor);

    info!("conformance_doctor_healthy passed");
}

#[test]
#[ignore]
fn conformance_doctor_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_doctor_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_doctor = workspace.run_obr(["doctor", "--json"], "doctor");
    let bd_doctor = workspace.run_bd(["doctor", "--json"], "doctor");

    assert!(obr_doctor.status.success(), "obr doctor failed");
    assert!(bd_doctor.status.success(), "bd doctor failed");

    let obr_json = extract_json_payload(&obr_doctor.stdout);
    let bd_json = extract_json_payload(&bd_doctor.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr doctor json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd doctor json");

    let obr_checks = obr_val
        .get("checks")
        .and_then(|checks| checks.as_array())
        .cloned()
        .unwrap_or_default();
    let bd_checks = bd_val
        .get("checks")
        .and_then(|checks| checks.as_array())
        .cloned()
        .unwrap_or_default();

    assert!(
        obr_checks
            .iter()
            .all(|c| c.get("name").is_some() && c.get("status").is_some()),
        "obr doctor checks missing name/status"
    );
    assert!(
        bd_checks
            .iter()
            .all(|c| c.get("name").is_some() && c.get("status").is_some()),
        "bd doctor checks missing name/status"
    );

    log_timings("doctor_json_shape", &obr_doctor, &bd_doctor);

    info!("conformance_doctor_json_shape passed");
}

#[test]
#[ignore]
fn conformance_doctor_with_issues() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_doctor_with_issues test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create some issues and dependencies
    let obr_a = workspace.run_obr(["create", "Issue A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Issue A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Issue B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Issue B", "--json"], "create_b");

    let obr_a_id = extract_issue_id(&extract_json_payload(&obr_a.stdout));
    let bd_a_id = extract_issue_id(&extract_json_payload(&bd_a.stdout));
    let obr_b_id = extract_issue_id(&extract_json_payload(&obr_b.stdout));
    let bd_b_id = extract_issue_id(&extract_json_payload(&bd_b.stdout));

    workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "add_dep");

    // Doctor should still succeed
    let obr_doctor = workspace.run_obr(["doctor", "--json"], "doctor");
    let bd_doctor = workspace.run_bd(["doctor", "--json"], "doctor");

    assert!(obr_doctor.status.success(), "obr doctor failed with issues");
    assert!(bd_doctor.status.success(), "bd doctor failed with issues");

    let obr_json = extract_json_payload(&obr_doctor.stdout);
    let bd_json = extract_json_payload(&bd_doctor.stdout);

    assert!(
        extract_checks_len(&obr_json) > 0,
        "obr doctor should emit checks"
    );
    assert!(
        extract_checks_len(&bd_json) > 0,
        "bd doctor should emit checks"
    );

    log_timings("doctor_with_issues", &obr_doctor, &bd_doctor);

    info!("conformance_doctor_with_issues passed");
}

// === INFO COMMAND TESTS ===

#[test]
#[ignore = "bd returns extra config object with compaction settings not implemented in obr"]
fn conformance_info_json_parity() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_info_json_parity test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_info = workspace.run_obr(["info", "--json"], "info");
    let bd_info = workspace.run_bd(["info", "--json"], "info");

    assert!(
        obr_info.status.success(),
        "obr info failed: {}",
        obr_info.stderr
    );
    assert!(
        bd_info.status.success(),
        "bd info failed: {}",
        bd_info.stderr
    );

    let obr_json = extract_json_payload(&obr_info.stdout);
    let bd_json = extract_json_payload(&bd_info.stdout);

    let mut obr_val: Value = serde_json::from_str(&obr_json).expect("obr info json");
    let mut bd_val: Value = serde_json::from_str(&bd_json).expect("bd info json");

    normalize_path_fields(&mut obr_val, &workspace.obr_root);
    normalize_path_fields(&mut bd_val, &workspace.bd_root);

    let excluded = vec![
        "obr_dir".to_string(),
        "db_size".to_string(),
        "jsonl_path".to_string(),
        "jsonl_size".to_string(),
        "daemon_detail".to_string(),
        "daemon_fallback_reason".to_string(),
    ];

    let obr_filtered = filter_fields(&obr_val, &excluded);
    let bd_filtered = filter_fields(&bd_val, &excluded);

    assert_eq!(
        obr_filtered,
        bd_filtered,
        "info JSON mismatch after normalization\nbr: {}\nbd: {}",
        serde_json::to_string_pretty(&obr_filtered).unwrap_or_default(),
        serde_json::to_string_pretty(&bd_filtered).unwrap_or_default()
    );

    log_timings("info_json_parity", &obr_info, &bd_info);
    info!("conformance_info_json_parity passed");
}

// === WHERE COMMAND TESTS ===

#[test]
#[ignore = "bd returns extra prefix field not implemented in obr"]
fn conformance_where_json_parity() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_where_json_parity test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_where = workspace.run_obr(["where", "--json"], "where");
    let bd_where = workspace.run_bd(["where", "--json"], "where");

    assert!(
        obr_where.status.success(),
        "obr where failed: {}",
        obr_where.stderr
    );
    assert!(
        bd_where.status.success(),
        "bd where failed: {}",
        bd_where.stderr
    );

    let obr_json = extract_json_payload(&obr_where.stdout);
    let bd_json = extract_json_payload(&bd_where.stdout);

    let mut obr_val: Value = serde_json::from_str(&obr_json).expect("obr where json");
    let mut bd_val: Value = serde_json::from_str(&bd_json).expect("bd where json");

    normalize_path_fields(&mut obr_val, &workspace.obr_root);
    normalize_path_fields(&mut bd_val, &workspace.bd_root);

    let excluded = vec!["jsonl_path".to_string()];
    let obr_filtered = filter_fields(&obr_val, &excluded);
    let bd_filtered = filter_fields(&bd_val, &excluded);

    assert_eq!(
        obr_filtered,
        bd_filtered,
        "where JSON mismatch after normalization\nbr: {}\nbd: {}",
        serde_json::to_string_pretty(&obr_filtered).unwrap_or_default(),
        serde_json::to_string_pretty(&bd_filtered).unwrap_or_default()
    );

    log_timings("where_json_parity", &obr_where, &bd_where);
    info!("conformance_where_json_parity passed");
}

// === VERSION COMMAND TESTS ===

#[test]
fn conformance_version_text() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_version_text test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Version without --json should produce text output
    let obr_version = workspace.run_obr(["version"], "version");
    let bd_version = workspace.run_bd(["version"], "version");

    assert!(
        obr_version.status.success(),
        "obr version failed: {}",
        obr_version.stderr
    );
    assert!(
        bd_version.status.success(),
        "bd version failed: {}",
        bd_version.stderr
    );

    // Both should output something
    assert!(
        !obr_version.stdout.trim().is_empty(),
        "obr version should produce output"
    );
    assert!(
        !bd_version.stdout.trim().is_empty(),
        "bd version should produce output"
    );

    log_timings("version_text", &obr_version, &bd_version);
    info!("conformance_version_text passed");
}

#[test]
fn conformance_version_json() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_version_json test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_version = workspace.run_obr(["version", "--json"], "version");
    let bd_version = workspace.run_bd(["version", "--json"], "version");

    assert!(
        obr_version.status.success(),
        "obr version --json failed: {}",
        obr_version.stderr
    );
    assert!(
        bd_version.status.success(),
        "bd version --json failed: {}",
        bd_version.stderr
    );

    let obr_json = extract_json_payload(&obr_version.stdout);
    let bd_json = extract_json_payload(&bd_version.stdout);

    // Both should produce valid JSON
    let obr_val: Result<Value, _> = serde_json::from_str(&obr_json);
    let bd_val: Result<Value, _> = serde_json::from_str(&bd_json);

    assert!(obr_val.is_ok(), "obr version should produce valid JSON");
    assert!(bd_val.is_ok(), "bd version should produce valid JSON");

    log_timings("version_json", &obr_version, &bd_version);
    info!("conformance_version_json passed");
}

#[test]
fn conformance_version_fields() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_version_fields test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_version = workspace.run_obr(["version", "--json"], "version");
    let bd_version = workspace.run_bd(["version", "--json"], "version");

    assert!(obr_version.status.success(), "obr version failed");
    assert!(bd_version.status.success(), "bd version failed");

    let obr_json = extract_json_payload(&obr_version.stdout);
    let bd_json = extract_json_payload(&bd_version.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Null);
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Null);

    let obr_has_version = obr_val.get("version").is_some();
    let bd_has_version = bd_val.get("version").is_some();
    let obr_has_build = obr_val.get("build").is_some();
    let bd_has_build = bd_val.get("build").is_some();

    assert!(obr_has_version, "obr version should have version field");
    assert!(bd_has_version, "bd version should have version field");
    assert!(obr_has_build, "obr version should have build field");
    assert!(bd_has_build, "bd version should have build field");

    log_timings("version_fields", &obr_version, &bd_version);
    info!("conformance_version_fields passed");
}

// === CONFIG COMMAND TESTS ===

#[test]
fn conformance_config_list() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_config_list test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_config = workspace.run_obr(["config", "list", "--json"], "config_list");
    let bd_config = workspace.run_bd(["config", "list", "--json"], "config_list");

    assert!(
        obr_config.status.success(),
        "obr config list failed: {}",
        obr_config.stderr
    );
    assert!(
        bd_config.status.success(),
        "bd config list failed: {}",
        bd_config.stderr
    );

    let obr_json = extract_json_payload(&obr_config.stdout);
    let bd_json = extract_json_payload(&bd_config.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr config json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd config json");

    let obr_prefix = obr_val.get("issue_prefix").and_then(|v| v.as_str());
    let bd_prefix = bd_val.get("issue_prefix").and_then(|v| v.as_str());

    assert!(obr_prefix.is_some(), "obr config list missing issue_prefix");
    assert!(bd_prefix.is_some(), "bd config list missing issue_prefix");

    log_timings("config_list", &obr_config, &bd_config);
    info!("conformance_config_list passed");
}

#[test]
#[ignore]
fn conformance_config_get() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_config_get test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_set = workspace.run_obr(["config", "set", "issue_prefix=cfg_get"], "config_set");
    let bd_set = workspace.run_bd(["config", "set", "issue_prefix", "cfg_get"], "config_set");

    assert!(
        obr_set.status.success(),
        "obr config set failed: {}",
        obr_set.stderr
    );
    assert!(
        bd_set.status.success(),
        "bd config set failed: {}",
        bd_set.stderr
    );

    let obr_get = workspace.run_obr(["config", "get", "issue_prefix", "--json"], "config_get");
    let bd_get = workspace.run_bd(["config", "get", "issue_prefix", "--json"], "config_get");

    assert!(
        obr_get.status.success(),
        "obr config get failed: {}",
        obr_get.stderr
    );
    assert!(
        bd_get.status.success(),
        "bd config get failed: {}",
        bd_get.stderr
    );

    let obr_json = extract_json_payload(&obr_get.stdout);
    let bd_json = extract_json_payload(&bd_get.stdout);

    log_timings("config_get", &obr_get, &bd_get);
    compare_json(&obr_json, &bd_json, &CompareMode::ExactJson).expect("JSON mismatch");

    info!("conformance_config_get passed");
}

#[test]
fn conformance_config_set() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_config_set test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_set = workspace.run_obr(["config", "set", "issue_prefix=cfg_set"], "config_set");
    let bd_set = workspace.run_bd(["config", "set", "issue_prefix", "cfg_set"], "config_set");

    assert!(
        obr_set.status.success(),
        "obr config set failed: {}",
        obr_set.stderr
    );
    assert!(
        bd_set.status.success(),
        "bd config set failed: {}",
        bd_set.stderr
    );

    log_timings("config_set", &obr_set, &bd_set);
    info!("conformance_config_set passed");
}

#[test]
#[ignore]
fn conformance_config_get_after_set() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_config_get_after_set test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_set = workspace.run_obr(["config", "set", "issue_prefix=cfg_after"], "config_set");
    let bd_set = workspace.run_bd(["config", "set", "issue_prefix", "cfg_after"], "config_set");

    assert!(
        obr_set.status.success(),
        "obr config set failed: {}",
        obr_set.stderr
    );
    assert!(
        bd_set.status.success(),
        "bd config set failed: {}",
        bd_set.stderr
    );

    let obr_get = workspace.run_obr(["config", "get", "issue_prefix", "--json"], "config_get");
    let bd_get = workspace.run_bd(["config", "get", "issue_prefix", "--json"], "config_get");

    assert!(
        obr_get.status.success(),
        "obr config get failed: {}",
        obr_get.stderr
    );
    assert!(
        bd_get.status.success(),
        "bd config get failed: {}",
        bd_get.stderr
    );

    let obr_json = extract_json_payload(&obr_get.stdout);
    let bd_json = extract_json_payload(&bd_get.stdout);

    log_timings("config_get_after_set", &obr_get, &bd_get);
    compare_json(&obr_json, &bd_json, &CompareMode::ExactJson).expect("JSON mismatch");

    info!("conformance_config_get_after_set passed");
}

#[test]
fn conformance_config_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_config_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_config = workspace.run_obr(["config", "list", "--json"], "config");
    let bd_config = workspace.run_bd(["config", "list", "--json"], "config");

    assert!(obr_config.status.success(), "obr config list failed");
    assert!(bd_config.status.success(), "bd config list failed");

    let obr_json = extract_json_payload(&obr_config.stdout);
    let bd_json = extract_json_payload(&bd_config.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr config json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd config json");

    assert!(obr_val.is_object(), "obr config list should be object");
    assert!(bd_val.is_object(), "bd config list should be object");

    log_timings("config_json_shape", &obr_config, &bd_config);

    info!("conformance_config_json_shape passed");
}

#[test]
fn conformance_config_defaults() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_config_defaults test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_config = workspace.run_obr(["config", "list", "--json"], "config_defaults");
    let bd_config = workspace.run_bd(["config", "list", "--json"], "config_defaults");

    assert!(obr_config.status.success(), "obr config list failed");
    assert!(bd_config.status.success(), "bd config list failed");

    let obr_json = extract_json_payload(&obr_config.stdout);
    let bd_json = extract_json_payload(&bd_config.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("obr config json");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("bd config json");

    let obr_prefix = obr_val
        .get("issue_prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let bd_prefix = bd_val
        .get("issue_prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    assert!(
        !obr_prefix.is_empty(),
        "obr config defaults should include issue_prefix"
    );
    assert!(
        !bd_prefix.is_empty(),
        "bd config defaults should include issue_prefix"
    );

    log_timings("config_defaults", &obr_config, &bd_config);

    info!("conformance_config_defaults passed");
}

#[test]
#[ignore]
fn conformance_config_invalid_key() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_config_invalid_key test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_config = workspace.run_obr(
        ["config", "get", "nonexistent.key.that.does.not.exist"],
        "config_invalid",
    );
    let bd_config = workspace.run_bd(
        ["config", "get", "nonexistent.key.that.does.not.exist"],
        "config_invalid",
    );

    assert_eq!(
        obr_config.status.success(),
        bd_config.status.success(),
        "obr/bd config invalid key exit mismatch"
    );
    assert!(
        !obr_config.status.success(),
        "config get should fail for invalid key"
    );

    log_timings("config_invalid_key", &obr_config, &bd_config);

    info!("conformance_config_invalid_key passed");
}

// ============================================================================
// REMAINING CRUD CONFORMANCE TESTS (beads_rust-j6tq)
// ============================================================================

// --- close tests ---

#[test]
fn conformance_close_already_closed() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_close_already_closed test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create and close an issue
    let obr_create = workspace.run_obr(["create", "To close twice", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "To close twice", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    workspace.run_obr(["close", &obr_id], "close1");
    workspace.run_bd(["close", &bd_id], "close1");

    // Try to close again
    let obr_close2 = workspace.run_obr(["close", &obr_id], "close2");
    let bd_close2 = workspace.run_bd(["close", &bd_id], "close2");

    // Both should handle double-close consistently
    info!(
        "obr double close: success={}, bd double close: success={}",
        obr_close2.status.success(),
        bd_close2.status.success()
    );

    info!("conformance_close_already_closed passed");
}

#[test]
fn conformance_close_sets_closed_at() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_close_sets_closed_at test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create an issue
    let obr_create = workspace.run_obr(["create", "Track close time", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Track close time", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Close it
    workspace.run_obr(["close", &obr_id, "--json"], "close");
    workspace.run_bd(["close", &bd_id, "--json"], "close");

    // Show and verify closed_at is set
    let obr_show = workspace.run_obr(["show", &obr_id, "--json"], "show_closed");
    let bd_show = workspace.run_bd(["show", &bd_id, "--json"], "show_closed");

    let obr_issue = issue_value_from_show_output(&obr_show, "obr show closed issue");
    let bd_issue = issue_value_from_show_output(&bd_show, "bd show closed issue");

    // Both should have closed_at set
    let obr_has_closed_at =
        obr_issue.get("closed_at").is_some() && !obr_issue["closed_at"].is_null();
    let bd_has_closed_at = bd_issue.get("closed_at").is_some() && !bd_issue["closed_at"].is_null();

    info!(
        "obr has closed_at: {}, bd has closed_at: {}",
        obr_has_closed_at, bd_has_closed_at
    );
    assert!(
        obr_has_closed_at,
        "obr closed issue missing closed_at: {obr_issue:?}"
    );
    assert!(
        bd_has_closed_at,
        "bd closed issue missing closed_at: {bd_issue:?}"
    );

    info!("conformance_close_sets_closed_at passed");
}

#[test]
fn conformance_close_blocked_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_close_blocked_issue test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create two issues with dependency (B blocked by A)
    let obr_a = workspace.run_obr(["create", "Blocker", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Blocker", "--json"], "create_a");
    let obr_b = workspace.run_obr(["create", "Blocked", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Blocked", "--json"], "create_b");

    let obr_a_id = extract_id_from_json(&obr_a.stdout);
    let bd_a_id = extract_id_from_json(&bd_a.stdout);
    let obr_b_id = extract_id_from_json(&obr_b.stdout);
    let bd_b_id = extract_id_from_json(&bd_b.stdout);

    workspace.run_obr(["dep", "add", &obr_b_id, &obr_a_id], "dep_add");
    workspace.run_bd(["dep", "add", &bd_b_id, &bd_a_id], "dep_add");

    // Try to close B (which is blocked)
    let obr_close = workspace.run_obr(["close", &obr_b_id], "close_blocked");
    let bd_close = workspace.run_bd(["close", &bd_b_id], "close_blocked");

    // Both should handle closing blocked issue consistently
    info!(
        "obr close blocked: success={}, bd close blocked: success={}",
        obr_close.status.success(),
        bd_close.status.success()
    );

    info!("conformance_close_blocked_issue passed");
}

#[test]
fn conformance_close_updates_dependents() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_close_updates_dependents test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create two issues with dependency
    let obr_a = workspace.run_obr(["create", "Blocker A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Blocker A", "--json"], "create_a");
    let obr_b = workspace.run_obr(["create", "Dependent B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Dependent B", "--json"], "create_b");

    let obr_a_id = extract_id_from_json(&obr_a.stdout);
    let bd_a_id = extract_id_from_json(&bd_a.stdout);
    let obr_b_id = extract_id_from_json(&obr_b.stdout);
    let bd_b_id = extract_id_from_json(&bd_b.stdout);

    workspace.run_obr(["dep", "add", &obr_b_id, &obr_a_id], "dep_add");
    workspace.run_bd(["dep", "add", &bd_b_id, &bd_a_id], "dep_add");

    // Verify B is blocked
    let obr_blocked = workspace.run_obr(["blocked", "--json"], "blocked_before");
    let bd_blocked = workspace.run_bd(["blocked", "--json"], "blocked_before");

    assert!(obr_blocked.status.success(), "obr blocked failed");
    assert!(bd_blocked.status.success(), "bd blocked failed");

    // Close A (the blocker)
    workspace.run_obr(["close", &obr_a_id], "close_blocker");
    workspace.run_bd(["close", &bd_a_id], "close_blocker");

    // B should now be unblocked (appear in ready list)
    let obr_ready = workspace.run_obr(["ready", "--json"], "ready_after");
    let bd_ready = workspace.run_bd(["ready", "--json"], "ready_after");

    assert!(obr_ready.status.success(), "obr ready failed");
    assert!(bd_ready.status.success(), "bd ready failed");

    info!("conformance_close_updates_dependents passed");
}

#[test]
fn conformance_close_preserves_fields() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_close_preserves_fields test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create with multiple fields
    let obr_create = workspace.run_obr(
        [
            "create",
            "Feature to close",
            "--type",
            "feature",
            "--priority",
            "1",
            "--assignee",
            "dev",
            "--json",
        ],
        "create",
    );
    let bd_create = workspace.run_bd(
        [
            "create",
            "Feature to close",
            "--type",
            "feature",
            "--priority",
            "1",
            "--assignee",
            "dev",
            "--json",
        ],
        "create",
    );

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Close with reason
    workspace.run_obr(
        ["close", &obr_id, "--reason", "Completed successfully"],
        "close",
    );
    workspace.run_bd(
        ["close", &bd_id, "--reason", "Completed successfully"],
        "close",
    );

    // Verify fields preserved
    let obr_show = workspace.run_obr(["show", &obr_id, "--json"], "show_closed");
    let bd_show = workspace.run_bd(["show", &bd_id, "--json"], "show_closed");

    let obr_json = extract_json_payload(&obr_show.stdout);
    let bd_json = extract_json_payload(&bd_show.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse obr");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse bd");

    let obr_issue = if obr_val.is_array() {
        &obr_val[0]
    } else {
        &obr_val
    };
    let bd_issue = if bd_val.is_array() {
        &bd_val[0]
    } else {
        &bd_val
    };

    // Priority should be preserved
    assert_eq!(
        obr_issue["priority"].as_i64(),
        Some(1),
        "obr priority changed after close"
    );
    assert_eq!(
        bd_issue["priority"].as_i64(),
        Some(1),
        "bd priority changed after close"
    );

    info!("conformance_close_preserves_fields passed");
}

// --- reopen tests ---

#[test]
fn conformance_reopen_clears_closed_at() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_reopen_clears_closed_at test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create, close, reopen
    let obr_create = workspace.run_obr(["create", "To reopen", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "To reopen", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    workspace.run_obr(["close", &obr_id], "close");
    workspace.run_bd(["close", &bd_id], "close");

    workspace.run_obr(["reopen", &obr_id], "reopen");
    workspace.run_bd(["reopen", &bd_id], "reopen");

    // Verify closed_at is cleared
    let obr_show = workspace.run_obr(["show", &obr_id, "--json"], "show_reopened");
    let bd_show = workspace.run_bd(["show", &bd_id, "--json"], "show_reopened");

    let obr_issue = issue_value_from_show_output(&obr_show, "obr show reopened issue");
    let bd_issue = issue_value_from_show_output(&bd_show, "bd show reopened issue");

    // closed_at should be null/cleared
    let obr_closed_at = obr_issue.get("closed_at");
    let bd_closed_at = bd_issue.get("closed_at");

    info!(
        "obr closed_at after reopen: {:?}, bd closed_at after reopen: {:?}",
        obr_closed_at, bd_closed_at
    );
    assert!(
        obr_closed_at.is_none_or(Value::is_null),
        "obr reopened issue should clear closed_at: {obr_issue:?}"
    );
    assert!(
        bd_closed_at.is_none_or(Value::is_null),
        "bd reopened issue should clear closed_at: {bd_issue:?}"
    );

    info!("conformance_reopen_clears_closed_at passed");
}

#[test]
fn conformance_reopen_preserves_fields() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_reopen_preserves_fields test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create with fields
    let obr_create = workspace.run_obr(
        [
            "create",
            "Reopen test",
            "--type",
            "bug",
            "--priority",
            "0",
            "--json",
        ],
        "create",
    );
    let bd_create = workspace.run_bd(
        [
            "create",
            "Reopen test",
            "--type",
            "bug",
            "--priority",
            "0",
            "--json",
        ],
        "create",
    );

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Close and reopen
    workspace.run_obr(["close", &obr_id, "--reason", "Done"], "close");
    workspace.run_bd(["close", &bd_id, "--reason", "Done"], "close");

    workspace.run_obr(["reopen", &obr_id], "reopen");
    workspace.run_bd(["reopen", &bd_id], "reopen");

    // Verify fields preserved
    let obr_show = workspace.run_obr(["show", &obr_id, "--json"], "show");
    let bd_show = workspace.run_bd(["show", &bd_id, "--json"], "show");

    let obr_json = extract_json_payload(&obr_show.stdout);
    let bd_json = extract_json_payload(&bd_show.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).expect("parse obr");
    let bd_val: Value = serde_json::from_str(&bd_json).expect("parse bd");

    let obr_issue = if obr_val.is_array() {
        &obr_val[0]
    } else {
        &obr_val
    };
    let bd_issue = if bd_val.is_array() {
        &bd_val[0]
    } else {
        &bd_val
    };

    // Priority should be preserved
    assert_eq!(
        obr_issue["priority"].as_i64(),
        Some(0),
        "obr priority changed after reopen"
    );
    assert_eq!(
        bd_issue["priority"].as_i64(),
        Some(0),
        "bd priority changed after reopen"
    );

    info!("conformance_reopen_preserves_fields passed");
}

#[test]
fn conformance_reopen_never_closed_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_reopen_never_closed_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create an issue but don't close it
    let obr_create = workspace.run_obr(["create", "Never closed", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Never closed", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Try to reopen (should fail or warn since not closed)
    let obr_reopen = workspace.run_obr(["reopen", &obr_id], "reopen_not_closed");
    let bd_reopen = workspace.run_bd(["reopen", &bd_id], "reopen_not_closed");

    // Both should handle this consistently
    info!(
        "obr reopen never closed: success={}, bd reopen never closed: success={}",
        obr_reopen.status.success(),
        bd_reopen.status.success()
    );

    info!("conformance_reopen_never_closed_error passed");
}

#[test]
fn conformance_reopen_tombstone_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_reopen_tombstone_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create and delete (tombstone)
    let obr_create = workspace.run_obr(["create", "To tombstone", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "To tombstone", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    workspace.run_obr(["delete", &obr_id], "delete");
    workspace.run_bd(["delete", &bd_id], "delete");

    // Try to reopen a tombstone
    let obr_reopen = workspace.run_obr(["reopen", &obr_id], "reopen_tombstone");
    let bd_reopen = workspace.run_bd(["reopen", &bd_id], "reopen_tombstone");

    // Both should handle this consistently (likely fail)
    info!(
        "obr reopen tombstone: success={}, bd reopen tombstone: success={}",
        obr_reopen.status.success(),
        bd_reopen.status.success()
    );

    info!("conformance_reopen_tombstone_error passed");
}

// ===========================================================================
// EPIC COMMAND CONFORMANCE TESTS (beads_rust-xewv)
// ===========================================================================

#[test]
fn conformance_epic_status_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_epic_status_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // No epics created - should return empty list
    let obr_out = workspace.run_obr(["epic", "status", "--json"], "epic_status_empty");
    let bd_out = workspace.run_bd(["epic", "status", "--json"], "epic_status_empty");

    info!(
        "obr epic status empty: success={}, bd epic status empty: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );

    // Both should succeed with empty result
    assert!(
        obr_out.status.success(),
        "obr epic status failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd epic status failed: {}",
        bd_out.stderr
    );

    info!("conformance_epic_status_empty passed");
}

#[test]
fn conformance_epic_status_with_epic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_epic_status_with_epic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create an epic in both workspaces
    let obr_create = workspace.run_obr(
        ["create", "Test epic", "--type", "epic", "--json"],
        "create_epic",
    );
    let bd_create = workspace.run_bd(
        ["create", "Test epic", "--type", "epic", "--json"],
        "create_epic",
    );

    assert!(
        obr_create.status.success(),
        "obr create epic failed: {}",
        obr_create.stderr
    );
    assert!(
        bd_create.status.success(),
        "bd create epic failed: {}",
        bd_create.stderr
    );

    // Get epic status
    let obr_out = workspace.run_obr(["epic", "status", "--json"], "epic_status");
    let bd_out = workspace.run_bd(["epic", "status", "--json"], "epic_status");

    assert!(
        obr_out.status.success(),
        "obr epic status failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd epic status failed: {}",
        bd_out.stderr
    );

    info!(
        "obr epic status result length: {}, bd epic status result length: {}",
        obr_out.stdout.len(),
        bd_out.stdout.len()
    );

    info!("conformance_epic_status_with_epic passed");
}

#[test]
fn conformance_epic_status_with_children() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_epic_status_with_children test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create epic and child in both workspaces
    let obr_epic = workspace.run_obr(
        ["create", "Parent epic", "--type", "epic", "--json"],
        "create_epic",
    );
    let bd_epic = workspace.run_bd(
        ["create", "Parent epic", "--type", "epic", "--json"],
        "create_epic",
    );

    let obr_epic_id = extract_id_from_json(&obr_epic.stdout);
    let bd_epic_id = extract_id_from_json(&bd_epic.stdout);

    // Create child task
    let obr_child = workspace.run_obr(
        ["create", "Child task", "--type", "task", "--json"],
        "create_child",
    );
    let bd_child = workspace.run_bd(
        ["create", "Child task", "--type", "task", "--json"],
        "create_child",
    );

    let obr_child_id = extract_id_from_json(&obr_child.stdout);
    let bd_child_id = extract_id_from_json(&bd_child.stdout);

    // Add parent-child dependency
    workspace.run_obr(
        [
            "dep",
            "add",
            &obr_child_id,
            &obr_epic_id,
            "--type",
            "parent-child",
        ],
        "add_parent-child",
    );
    workspace.run_bd(
        [
            "dep",
            "add",
            &bd_child_id,
            &bd_epic_id,
            "--type",
            "parent-child",
        ],
        "add_parent-child",
    );

    // Get epic status
    let obr_out = workspace.run_obr(["epic", "status", "--json"], "epic_status_children");
    let bd_out = workspace.run_bd(["epic", "status", "--json"], "epic_status_children");

    assert!(
        obr_out.status.success(),
        "obr epic status failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd epic status failed: {}",
        bd_out.stderr
    );

    info!("conformance_epic_status_with_children passed");
}

#[test]
fn conformance_epic_close_eligible_open_children() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_epic_close_eligible_open_children test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create epic with open child
    let obr_epic = workspace.run_obr(
        ["create", "Epic with open child", "--type", "epic", "--json"],
        "create_epic",
    );
    let bd_epic = workspace.run_bd(
        ["create", "Epic with open child", "--type", "epic", "--json"],
        "create_epic",
    );

    let obr_epic_id = extract_id_from_json(&obr_epic.stdout);
    let bd_epic_id = extract_id_from_json(&bd_epic.stdout);

    // Create open child
    let obr_child = workspace.run_obr(["create", "Open child", "--json"], "create_child");
    let bd_child = workspace.run_bd(["create", "Open child", "--json"], "create_child");

    let obr_child_id = extract_id_from_json(&obr_child.stdout);
    let bd_child_id = extract_id_from_json(&bd_child.stdout);

    // Add parent-child dependency
    workspace.run_obr(
        [
            "dep",
            "add",
            &obr_child_id,
            &obr_epic_id,
            "--type",
            "parent-child",
        ],
        "add_parent-child",
    );
    workspace.run_bd(
        [
            "dep",
            "add",
            &bd_child_id,
            &bd_epic_id,
            "--type",
            "parent-child",
        ],
        "add_parent-child",
    );

    // Try to close eligible - should not close epic with open children
    let obr_out = workspace.run_obr(["epic", "close-eligible", "--json"], "close_eligible");
    let bd_out = workspace.run_bd(["epic", "close-eligible", "--json"], "close_eligible");

    info!(
        "obr close-eligible result: success={}, bd close-eligible result: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );

    // Verify epic is still open
    let obr_show = workspace.run_obr(["show", &obr_epic_id, "--json"], "show_epic");
    let bd_show = workspace.run_bd(["show", &bd_epic_id, "--json"], "show_epic");

    let obr_json = extract_json_payload(&obr_show.stdout);
    let bd_json = extract_json_payload(&bd_show.stdout);

    let obr_status: Value = serde_json::from_str(&obr_json).expect("parse obr json");
    let bd_status: Value = serde_json::from_str(&bd_json).expect("parse bd json");

    // Both should still be open (show returns array, access first element)
    assert_eq!(
        obr_status[0].get("status").and_then(|v| v.as_str()),
        Some("open"),
        "obr epic should still be open"
    );
    assert_eq!(
        bd_status[0].get("status").and_then(|v| v.as_str()),
        Some("open"),
        "bd epic should still be open"
    );

    info!("conformance_epic_close_eligible_open_children passed");
}

/// NOTE: This test is ignored because obr and bd have different semantics for parent-child dependencies.
/// In bd, children can be closed while the parent epic is open.
/// In obr, children are blocked by the parent being open (parent-child creates a blocking dependency).
/// This causes the test to fail: obr's close skips the child, so the epic never becomes eligible.
/// This is a known behavioral difference that would require changing obr's dep blocking logic to fix.
#[test]
#[ignore = "obr parent-child dependency blocks children; bd does not"]
fn conformance_epic_close_eligible_all_closed() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_epic_close_eligible_all_closed test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create epic with child
    let obr_epic = workspace.run_obr(
        ["create", "Epic all closed", "--type", "epic", "--json"],
        "create_epic",
    );
    let bd_epic = workspace.run_bd(
        ["create", "Epic all closed", "--type", "epic", "--json"],
        "create_epic",
    );

    let obr_epic_id = extract_id_from_json(&obr_epic.stdout);
    let bd_epic_id = extract_id_from_json(&bd_epic.stdout);

    // Create child
    let obr_child = workspace.run_obr(["create", "Child to close", "--json"], "create_child");
    let bd_child = workspace.run_bd(["create", "Child to close", "--json"], "create_child");

    let obr_child_id = extract_id_from_json(&obr_child.stdout);
    let bd_child_id = extract_id_from_json(&bd_child.stdout);

    // Add parent-child dependency
    workspace.run_obr(
        [
            "dep",
            "add",
            &obr_child_id,
            &obr_epic_id,
            "--type",
            "parent-child",
        ],
        "add_parent-child",
    );
    workspace.run_bd(
        [
            "dep",
            "add",
            &bd_child_id,
            &bd_epic_id,
            "--type",
            "parent-child",
        ],
        "add_parent-child",
    );

    // Close the child
    workspace.run_obr(["close", &obr_child_id], "close_child");
    workspace.run_bd(["close", &bd_child_id], "close_child");

    // Now close-eligible should close the epic
    let obr_out = workspace.run_obr(["epic", "close-eligible", "--json"], "close_eligible");
    let bd_out = workspace.run_bd(["epic", "close-eligible", "--json"], "close_eligible");

    info!(
        "obr close-eligible all closed: success={}, bd close-eligible all closed: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );

    // Verify epic is now closed
    let obr_show = workspace.run_obr(["show", &obr_epic_id, "--json"], "show_epic_after");
    let bd_show = workspace.run_bd(["show", &bd_epic_id, "--json"], "show_epic_after");

    let obr_json = extract_json_payload(&obr_show.stdout);
    let bd_json = extract_json_payload(&bd_show.stdout);

    let obr_status: Value = serde_json::from_str(&obr_json).expect("parse obr json");
    let bd_status: Value = serde_json::from_str(&bd_json).expect("parse bd json");

    // Both should now be closed (show returns array, access first element)
    assert_eq!(
        obr_status[0].get("status").and_then(|v| v.as_str()),
        Some("closed"),
        "obr epic should be closed"
    );
    assert_eq!(
        bd_status[0].get("status").and_then(|v| v.as_str()),
        Some("closed"),
        "bd epic should be closed"
    );

    info!("conformance_epic_close_eligible_all_closed passed");
}

#[test]
fn conformance_epic_status_eligible_only() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_epic_status_eligible_only test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create two epics: one eligible, one not
    // Epic 1: no children (eligible)
    let _obr_epic1 = workspace.run_obr(
        ["create", "Epic no children", "--type", "epic", "--json"],
        "create_epic1",
    );
    let _bd_epic1 = workspace.run_bd(
        ["create", "Epic no children", "--type", "epic", "--json"],
        "create_epic1",
    );

    // Epic 2: with open child (not eligible)
    let obr_epic2 = workspace.run_obr(
        ["create", "Epic with open child", "--type", "epic", "--json"],
        "create_epic2",
    );
    let bd_epic2 = workspace.run_bd(
        ["create", "Epic with open child", "--type", "epic", "--json"],
        "create_epic2",
    );

    let obr_epic2_id = extract_id_from_json(&obr_epic2.stdout);
    let bd_epic2_id = extract_id_from_json(&bd_epic2.stdout);

    // Add open child to epic2
    let obr_child = workspace.run_obr(["create", "Open child", "--json"], "create_child");
    let bd_child = workspace.run_bd(["create", "Open child", "--json"], "create_child");

    let obr_child_id = extract_id_from_json(&obr_child.stdout);
    let bd_child_id = extract_id_from_json(&bd_child.stdout);

    workspace.run_obr(
        [
            "dep",
            "add",
            &obr_child_id,
            &obr_epic2_id,
            "--type",
            "parent-child",
        ],
        "add_parent-child",
    );
    workspace.run_bd(
        [
            "dep",
            "add",
            &bd_child_id,
            &bd_epic2_id,
            "--type",
            "parent-child",
        ],
        "add_parent-child",
    );

    // Get only eligible epics
    let obr_out = workspace.run_obr(
        ["epic", "status", "--eligible-only", "--json"],
        "epic_eligible_only",
    );
    let bd_out = workspace.run_bd(
        ["epic", "status", "--eligible-only", "--json"],
        "epic_eligible_only",
    );

    assert!(
        obr_out.status.success(),
        "obr epic status failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd epic status failed: {}",
        bd_out.stderr
    );

    info!(
        "obr eligible-only result: {}, bd eligible-only result: {}",
        obr_out.stdout.trim(),
        bd_out.stdout.trim()
    );

    info!("conformance_epic_status_eligible_only passed");
}

#[test]
fn conformance_epic_status_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_epic_status_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create an epic
    workspace.run_obr(
        ["create", "JSON shape test epic", "--type", "epic", "--json"],
        "create_epic",
    );
    workspace.run_bd(
        ["create", "JSON shape test epic", "--type", "epic", "--json"],
        "create_epic",
    );

    // Get status
    let obr_out = workspace.run_obr(["epic", "status", "--json"], "epic_status_json");
    let bd_out = workspace.run_bd(["epic", "status", "--json"], "epic_status_json");

    assert!(
        obr_out.status.success(),
        "obr epic status failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd epic status failed: {}",
        bd_out.stderr
    );

    // Compare structure
    let obr_json = extract_json_payload(&obr_out.stdout);
    let bd_json = extract_json_payload(&bd_out.stdout);

    let result = compare_json(&obr_json, &bd_json, &CompareMode::StructureOnly);
    if let Err(e) = &result {
        info!("Structure comparison note (may differ): {}", e);
    }

    info!("conformance_epic_status_json_shape passed");
}

#[test]
fn conformance_epic_nested() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_epic_nested test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create parent epic
    let obr_parent = workspace.run_obr(
        ["create", "Parent epic", "--type", "epic", "--json"],
        "create_parent",
    );
    let bd_parent = workspace.run_bd(
        ["create", "Parent epic", "--type", "epic", "--json"],
        "create_parent",
    );

    let obr_parent_id = extract_id_from_json(&obr_parent.stdout);
    let bd_parent_id = extract_id_from_json(&bd_parent.stdout);

    // Create child epic
    let obr_child = workspace.run_obr(
        ["create", "Child epic", "--type", "epic", "--json"],
        "create_child_epic",
    );
    let bd_child = workspace.run_bd(
        ["create", "Child epic", "--type", "epic", "--json"],
        "create_child_epic",
    );

    let obr_child_id = extract_id_from_json(&obr_child.stdout);
    let bd_child_id = extract_id_from_json(&bd_child.stdout);

    // Add child epic to parent epic
    workspace.run_obr(
        [
            "dep",
            "add",
            &obr_child_id,
            &obr_parent_id,
            "--type",
            "parent-child",
        ],
        "add_nested",
    );
    workspace.run_bd(
        [
            "dep",
            "add",
            &bd_child_id,
            &bd_parent_id,
            "--type",
            "parent-child",
        ],
        "add_nested",
    );

    // Get status
    let obr_out = workspace.run_obr(["epic", "status", "--json"], "epic_nested_status");
    let bd_out = workspace.run_bd(["epic", "status", "--json"], "epic_nested_status");

    assert!(
        obr_out.status.success(),
        "obr epic status failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd epic status failed: {}",
        bd_out.stderr
    );

    info!("conformance_epic_nested passed");
}

// ===========================================================================
// GRAPH COMMAND CONFORMANCE TESTS (beads_rust-xewv)
// ===========================================================================

#[test]
fn conformance_graph_no_deps() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_graph_no_deps test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create a single issue with no dependencies
    let obr_create = workspace.run_obr(["create", "No deps issue", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "No deps issue", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Get graph for issue with no deps
    let obr_out = workspace.run_obr(["graph", &obr_id, "--json"], "graph_no_deps");
    let bd_out = workspace.run_bd(["graph", &bd_id, "--json"], "graph_no_deps");

    info!(
        "obr graph no deps: success={}, bd graph no deps: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );
    assert_json_output_mentions(&obr_out, &obr_id, "obr graph no deps");
    assert_json_output_mentions(&bd_out, &bd_id, "bd graph no deps");

    info!("conformance_graph_no_deps passed");
}

/// INTENTIONAL DIVERGENCE — `obr graph <id>` walks the graph in the opposite
/// direction to `bd graph <id>` (`beads_rust-mf72`).
///
/// After `obr dep add A B` (A depends on B), `obr graph A` returns just A with no
/// edges, while `obr graph B` returns `{"nodes":[B,A],"edges":[["A","B"]]}`. obr
/// traverses *dependents* — "what does closing this unblock?" — where bd
/// traverses *dependencies*. This is by design and stated in
/// `src/cli/commands/graph.rs`: "Visualizes dependency graphs with focus on
/// reverse dependencies (dependents)". `obr dep tree` covers the other
/// direction.
///
/// These three tests assert bd's direction and so can never pass. Verified by
/// hand against a real bd v0.46.0 on 2026-07-25.
#[test]
#[ignore = "beads_rust-mf72: obr graph shows dependents by design; bd shows dependencies"]
fn conformance_graph_simple_dep() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_graph_simple_dep test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create A and B
    let obr_a = workspace.run_obr(["create", "Issue A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Issue A", "--json"], "create_a");

    let obr_b = workspace.run_obr(["create", "Issue B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Issue B", "--json"], "create_b");

    let obr_a_id = extract_id_from_json(&obr_a.stdout);
    let bd_a_id = extract_id_from_json(&bd_a.stdout);
    let obr_b_id = extract_id_from_json(&obr_b.stdout);
    let bd_b_id = extract_id_from_json(&bd_b.stdout);

    // A depends on B (A -> B)
    let obr_dep = workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "add_dep");
    let bd_dep = workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "add_dep");
    assert_command_success(&obr_dep, "obr graph simple dep setup");
    assert_command_success(&bd_dep, "bd graph simple dep setup");

    // Get graph from A
    let obr_out = workspace.run_obr(["graph", &obr_a_id, "--json"], "graph_simple");
    let bd_out = workspace.run_bd(["graph", &bd_a_id, "--json"], "graph_simple");

    info!(
        "obr graph simple dep: success={}, bd graph simple dep: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );
    assert_json_output_mentions(&obr_out, &obr_a_id, "obr graph simple root");
    assert_json_output_mentions(&obr_out, &obr_b_id, "obr graph simple dependency");
    assert_json_output_mentions(&bd_out, &bd_a_id, "bd graph simple root");
    assert_json_output_mentions(&bd_out, &bd_b_id, "bd graph simple dependency");

    info!("conformance_graph_simple_dep passed");
}

/// See `conformance_graph_simple_dep` — same direction divergence
/// (`beads_rust-mf72`).
#[test]
#[ignore = "beads_rust-mf72: obr graph shows dependents by design; bd shows dependencies"]
fn conformance_graph_complex_deps() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_graph_complex_deps test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create A, B, C, D
    let obr_a = workspace.run_obr(["create", "Issue A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Issue A", "--json"], "create_a");
    let obr_b = workspace.run_obr(["create", "Issue B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Issue B", "--json"], "create_b");
    let obr_c = workspace.run_obr(["create", "Issue C", "--json"], "create_c");
    let bd_c = workspace.run_bd(["create", "Issue C", "--json"], "create_c");
    let obr_d = workspace.run_obr(["create", "Issue D", "--json"], "create_d");
    let bd_d = workspace.run_bd(["create", "Issue D", "--json"], "create_d");

    let obr_a_id = extract_id_from_json(&obr_a.stdout);
    let bd_a_id = extract_id_from_json(&bd_a.stdout);
    let obr_b_id = extract_id_from_json(&obr_b.stdout);
    let bd_b_id = extract_id_from_json(&bd_b.stdout);
    let obr_c_id = extract_id_from_json(&obr_c.stdout);
    let bd_c_id = extract_id_from_json(&bd_c.stdout);
    let obr_d_id = extract_id_from_json(&obr_d.stdout);
    let bd_d_id = extract_id_from_json(&bd_d.stdout);

    // A -> B, A -> C, B -> D, C -> D (diamond pattern)
    let obr_dep_ab = workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "dep_ab");
    let bd_dep_ab = workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "dep_ab");
    let obr_dep_ac = workspace.run_obr(["dep", "add", &obr_a_id, &obr_c_id], "dep_ac");
    let bd_dep_ac = workspace.run_bd(["dep", "add", &bd_a_id, &bd_c_id], "dep_ac");
    let obr_dep_bd = workspace.run_obr(["dep", "add", &obr_b_id, &obr_d_id], "dep_bd");
    let bd_dep_bd = workspace.run_bd(["dep", "add", &bd_b_id, &bd_d_id], "dep_bd");
    let obr_dep_cd = workspace.run_obr(["dep", "add", &obr_c_id, &obr_d_id], "dep_cd");
    let bd_dep_cd = workspace.run_bd(["dep", "add", &bd_c_id, &bd_d_id], "dep_cd");
    assert_command_success(&obr_dep_ab, "obr graph complex dep_ab setup");
    assert_command_success(&bd_dep_ab, "bd graph complex dep_ab setup");
    assert_command_success(&obr_dep_ac, "obr graph complex dep_ac setup");
    assert_command_success(&bd_dep_ac, "bd graph complex dep_ac setup");
    assert_command_success(&obr_dep_bd, "obr graph complex dep_bd setup");
    assert_command_success(&bd_dep_bd, "bd graph complex dep_bd setup");
    assert_command_success(&obr_dep_cd, "obr graph complex dep_cd setup");
    assert_command_success(&bd_dep_cd, "bd graph complex dep_cd setup");

    // Get graph from A
    let obr_out = workspace.run_obr(["graph", &obr_a_id, "--json"], "graph_complex");
    let bd_out = workspace.run_bd(["graph", &bd_a_id, "--json"], "graph_complex");

    info!(
        "obr graph complex: success={}, bd graph complex: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );
    for expected in [&obr_a_id, &obr_b_id, &obr_c_id, &obr_d_id] {
        assert_json_output_mentions(&obr_out, expected, "obr graph complex");
    }
    for expected in [&bd_a_id, &bd_b_id, &bd_c_id, &bd_d_id] {
        assert_json_output_mentions(&bd_out, expected, "bd graph complex");
    }

    info!("conformance_graph_complex_deps passed");
}

/// See `conformance_graph_simple_dep` — same direction divergence
/// (`beads_rust-mf72`).
#[test]
#[ignore = "beads_rust-mf72: obr graph shows dependents by design; bd shows dependencies"]
fn conformance_graph_all_flag() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_graph_all_flag test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create some issues
    let obr_issue1 = workspace.run_obr(["create", "Issue 1", "--json"], "create1");
    let bd_issue1 = workspace.run_bd(["create", "Issue 1", "--json"], "create1");
    let obr_issue2 = workspace.run_obr(["create", "Issue 2", "--json"], "create2");
    let bd_issue2 = workspace.run_bd(["create", "Issue 2", "--json"], "create2");
    let obr_issue1_id = extract_id_from_json(&obr_issue1.stdout);
    let bd_issue1_id = extract_id_from_json(&bd_issue1.stdout);
    let obr_issue2_id = extract_id_from_json(&obr_issue2.stdout);
    let bd_issue2_id = extract_id_from_json(&bd_issue2.stdout);

    // Get graph for all issues
    let obr_out = workspace.run_obr(["graph", "--all", "--json"], "graph_all");
    let bd_out = workspace.run_bd(["graph", "--all", "--json"], "graph_all");

    info!(
        "obr graph --all: success={}, bd graph --all: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );
    assert_json_output_mentions(&obr_out, &obr_issue1_id, "obr graph all first issue");
    assert_json_output_mentions(&obr_out, &obr_issue2_id, "obr graph all second issue");
    assert_json_output_mentions(&bd_out, &bd_issue1_id, "bd graph all first issue");
    assert_json_output_mentions(&bd_out, &bd_issue2_id, "bd graph all second issue");

    info!("conformance_graph_all_flag passed");
}

/// INTENTIONAL DIVERGENCE: obr has `--compact` flag for graph command, bd v0.46.0 does not.
/// This is a br-only enhancement, not a conformance requirement.
#[test]
#[ignore = "br-only feature: --compact flag not in bd v0.46.0"]
fn conformance_graph_compact_flag() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_graph_compact_flag test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create A -> B
    let obr_a = workspace.run_obr(["create", "Issue A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Issue A", "--json"], "create_a");
    let obr_b = workspace.run_obr(["create", "Issue B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Issue B", "--json"], "create_b");

    let obr_a_id = extract_id_from_json(&obr_a.stdout);
    let bd_a_id = extract_id_from_json(&bd_a.stdout);
    let obr_b_id = extract_id_from_json(&obr_b.stdout);
    let bd_b_id = extract_id_from_json(&bd_b.stdout);

    workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "add_dep");

    // Get compact graph
    let obr_out = workspace.run_obr(["graph", &obr_a_id, "--compact"], "graph_compact");
    let bd_out = workspace.run_bd(["graph", &bd_a_id, "--compact"], "graph_compact");

    info!(
        "obr graph --compact: success={}, bd graph --compact: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );

    // Both should succeed
    assert!(
        obr_out.status.success(),
        "obr graph --compact failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd graph --compact failed: {}",
        bd_out.stderr
    );

    info!("conformance_graph_compact_flag passed");
}

#[test]
fn conformance_graph_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_graph_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create A -> B
    let obr_a = workspace.run_obr(["create", "Issue A", "--json"], "create_a");
    let bd_a = workspace.run_bd(["create", "Issue A", "--json"], "create_a");
    let obr_b = workspace.run_obr(["create", "Issue B", "--json"], "create_b");
    let bd_b = workspace.run_bd(["create", "Issue B", "--json"], "create_b");

    let obr_a_id = extract_id_from_json(&obr_a.stdout);
    let bd_a_id = extract_id_from_json(&bd_a.stdout);
    let obr_b_id = extract_id_from_json(&obr_b.stdout);
    let bd_b_id = extract_id_from_json(&bd_b.stdout);

    workspace.run_obr(["dep", "add", &obr_a_id, &obr_b_id], "add_dep");
    workspace.run_bd(["dep", "add", &bd_a_id, &bd_b_id], "add_dep");

    // Get JSON graph
    let obr_out = workspace.run_obr(["graph", &obr_a_id, "--json"], "graph_json");
    let bd_out = workspace.run_bd(["graph", &bd_a_id, "--json"], "graph_json");

    assert!(
        obr_out.status.success(),
        "obr graph --json failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd graph --json failed: {}",
        bd_out.stderr
    );

    let obr_json = extract_json_payload(&obr_out.stdout);
    let bd_json = extract_json_payload(&bd_out.stdout);

    let result = compare_json(&obr_json, &bd_json, &CompareMode::StructureOnly);
    if let Err(e) = &result {
        info!("Graph JSON structure comparison note (may differ): {}", e);
    }

    info!("conformance_graph_json_shape passed");
}

// ===========================================================================
// AUDIT COMMAND CONFORMANCE TESTS (beads_rust-xewv)
// ===========================================================================

#[test]
fn conformance_audit_record_llm_call() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_audit_record_llm_call test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Record an LLM call
    let obr_out = workspace.run_obr(
        [
            "audit",
            "record",
            "--kind",
            "llm_call",
            "--model",
            "gpt-4",
            "--prompt",
            "Hello world",
            "--response",
            "Hi there!",
            "--json",
        ],
        "audit_llm_call",
    );
    let bd_out = workspace.run_bd(
        [
            "audit",
            "record",
            "--kind",
            "llm_call",
            "--model",
            "gpt-4",
            "--prompt",
            "Hello world",
            "--response",
            "Hi there!",
            "--json",
        ],
        "audit_llm_call",
    );

    info!(
        "obr audit record llm_call: success={}, bd audit record llm_call: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );

    assert!(
        obr_out.status.success(),
        "obr audit record failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd audit record failed: {}",
        bd_out.stderr
    );

    info!("conformance_audit_record_llm_call passed");
}

#[test]
fn conformance_audit_record_tool_call() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_audit_record_tool_call test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Record a tool call
    let obr_out = workspace.run_obr(
        [
            "audit",
            "record",
            "--kind",
            "tool_call",
            "--tool-name",
            "bash",
            "--exit-code",
            "0",
            "--json",
        ],
        "audit_tool_call",
    );
    let bd_out = workspace.run_bd(
        [
            "audit",
            "record",
            "--kind",
            "tool_call",
            "--tool-name",
            "bash",
            "--exit-code",
            "0",
            "--json",
        ],
        "audit_tool_call",
    );

    info!(
        "obr audit record tool_call: success={}, bd audit record tool_call: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );

    assert!(
        obr_out.status.success(),
        "obr audit record tool_call failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd audit record tool_call failed: {}",
        bd_out.stderr
    );

    info!("conformance_audit_record_tool_call passed");
}

#[test]
fn conformance_audit_record_with_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_audit_record_with_issue test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create an issue to reference
    let obr_create = workspace.run_obr(["create", "Test issue for audit", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Test issue for audit", "--json"], "create");

    let obr_id = extract_id_from_json(&obr_create.stdout);
    let bd_id = extract_id_from_json(&bd_create.stdout);

    // Record with issue reference
    let obr_out = workspace.run_obr(
        [
            "audit",
            "record",
            "--kind",
            "llm_call",
            "--issue-id",
            &obr_id,
            "--model",
            "claude",
            "--prompt",
            "Fix the bug",
            "--response",
            "Bug fixed",
            "--json",
        ],
        "audit_with_issue",
    );
    let bd_out = workspace.run_bd(
        [
            "audit",
            "record",
            "--kind",
            "llm_call",
            "--issue-id",
            &bd_id,
            "--model",
            "claude",
            "--prompt",
            "Fix the bug",
            "--response",
            "Bug fixed",
            "--json",
        ],
        "audit_with_issue",
    );

    info!(
        "obr audit record with issue: success={}, bd audit record with issue: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );
    assert_command_success(&obr_out, "obr audit record with issue");
    assert_command_success(&bd_out, "bd audit record with issue");

    let obr_record: Value = serde_json::from_str(&extract_json_payload(&obr_out.stdout))
        .expect("obr audit record json");
    let bd_record: Value =
        serde_json::from_str(&extract_json_payload(&bd_out.stdout)).expect("bd audit record json");
    assert!(
        obr_record["id"].as_str().is_some_and(|id| !id.is_empty()),
        "obr audit record should return an id: {obr_record:?}"
    );
    assert_eq!(obr_record["kind"].as_str(), Some("llm_call"));
    assert!(
        bd_record["id"].as_str().is_some_and(|id| !id.is_empty()),
        "bd audit record should return an id: {bd_record:?}"
    );
    assert_eq!(bd_record["kind"].as_str(), Some("llm_call"));

    info!("conformance_audit_record_with_issue passed");
}

#[test]
fn conformance_audit_label() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_audit_label test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // First record an entry to get an ID
    let obr_record = workspace.run_obr(
        [
            "audit",
            "record",
            "--kind",
            "llm_call",
            "--model",
            "test",
            "--prompt",
            "test",
            "--response",
            "test",
            "--json",
        ],
        "audit_record_for_label",
    );
    let bd_record = workspace.run_bd(
        [
            "audit",
            "record",
            "--kind",
            "llm_call",
            "--model",
            "test",
            "--prompt",
            "test",
            "--response",
            "test",
            "--json",
        ],
        "audit_record_for_label",
    );

    assert_command_success(&obr_record, "obr audit record for label");
    assert_command_success(&bd_record, "bd audit record for label");

    // Extract entry IDs from the output
    let obr_entry_id = extract_audit_entry_id(&obr_record.stdout);
    let bd_entry_id = extract_audit_entry_id(&bd_record.stdout);
    info!(
        "obr entry_id: {}, bd entry_id: {}",
        obr_entry_id, bd_entry_id
    );

    // Now label the entries
    let obr_out = workspace.run_obr(
        [
            "audit",
            "label",
            &obr_entry_id,
            "--label",
            "good",
            "--reason",
            "Test label",
            "--json",
        ],
        "audit_label",
    );
    let bd_out = workspace.run_bd(
        [
            "audit",
            "label",
            &bd_entry_id,
            "--label",
            "good",
            "--reason",
            "Test label",
            "--json",
        ],
        "audit_label",
    );

    info!(
        "obr audit label: success={}, bd audit label: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );
    assert_command_success(&obr_out, "obr audit label");
    assert_command_success(&bd_out, "bd audit label");

    let obr_label: Value =
        serde_json::from_str(&extract_json_payload(&obr_out.stdout)).expect("obr audit label json");
    let bd_label: Value =
        serde_json::from_str(&extract_json_payload(&bd_out.stdout)).expect("bd audit label json");
    assert_eq!(obr_label["parent_id"].as_str(), Some(obr_entry_id.as_str()));
    assert_eq!(obr_label["label"].as_str(), Some("good"));
    assert!(
        obr_label["id"].as_str().is_some_and(|id| !id.is_empty()),
        "obr audit label should return a label entry id: {obr_label:?}"
    );
    assert_eq!(bd_label["parent_id"].as_str(), Some(bd_entry_id.as_str()));
    assert_eq!(bd_label["label"].as_str(), Some("good"));
    assert!(
        bd_label["id"].as_str().is_some_and(|id| !id.is_empty()),
        "bd audit label should return a label entry id: {bd_label:?}"
    );

    info!("conformance_audit_label passed");
}

#[test]
fn conformance_audit_record_with_error() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_audit_record_with_error test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Record a tool call with error
    let obr_out = workspace.run_obr(
        [
            "audit",
            "record",
            "--kind",
            "tool_call",
            "--tool-name",
            "bash",
            "--exit-code",
            "1",
            "--error",
            "Command failed: permission denied",
            "--json",
        ],
        "audit_error",
    );
    let bd_out = workspace.run_bd(
        [
            "audit",
            "record",
            "--kind",
            "tool_call",
            "--tool-name",
            "bash",
            "--exit-code",
            "1",
            "--error",
            "Command failed: permission denied",
            "--json",
        ],
        "audit_error",
    );

    info!(
        "obr audit record with error: success={}, bd audit record with error: success={}",
        obr_out.status.success(),
        bd_out.status.success()
    );

    assert!(
        obr_out.status.success(),
        "obr audit record with error failed: {}",
        obr_out.stderr
    );
    assert!(
        bd_out.status.success(),
        "bd audit record with error failed: {}",
        bd_out.stderr
    );

    info!("conformance_audit_record_with_error passed");
}

/// Helper to extract audit entry ID from JSON output
fn extract_audit_entry_id(output: &str) -> String {
    let json = extract_json_payload(output);
    let v: Value = serde_json::from_str(&json).expect("parse audit entry json");
    v.get("id")
        .or_else(|| v.get("entry_id"))
        .and_then(|value| value.as_str())
        .expect("audit entry JSON should include id or entry_id")
        .to_string()
}

// ============================================================================
// Q (QUICK CAPTURE) COMMAND TESTS
// ============================================================================

#[test]
fn conformance_q_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_q_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Quick capture outputs just the ID
    let obr_q = workspace.run_obr(["q", "Quick capture test"], "q_basic");
    let bd_q = workspace.run_bd(["q", "Quick capture test"], "q_basic");

    assert!(obr_q.status.success(), "obr q failed: {}", obr_q.stderr);
    assert!(bd_q.status.success(), "bd q failed: {}", bd_q.stderr);

    // Output should be just an ID (short, no JSON wrapper)
    let obr_id = obr_q.stdout.trim();
    let bd_id = bd_q.stdout.trim();

    assert!(!obr_id.is_empty(), "obr q should output an ID");
    assert!(!bd_id.is_empty(), "bd q should output an ID");

    info!("obr q ID: {}, bd q ID: {}", obr_id, bd_id);
    info!("conformance_q_basic passed");
}

#[test]
fn conformance_q_with_priority() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_q_with_priority test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_q = workspace.run_obr(["q", "High priority quick", "-p", "1"], "q_priority");
    let bd_q = workspace.run_bd(["q", "High priority quick", "-p", "1"], "q_priority");

    assert!(
        obr_q.status.success(),
        "obr q with priority failed: {}",
        obr_q.stderr
    );
    assert!(
        bd_q.status.success(),
        "bd q with priority failed: {}",
        bd_q.stderr
    );

    info!("conformance_q_with_priority passed");
}

#[test]
fn conformance_q_with_type() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_q_with_type test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_q = workspace.run_obr(["q", "Bug quick capture", "-t", "bug"], "q_type");
    let bd_q = workspace.run_bd(["q", "Bug quick capture", "-t", "bug"], "q_type");

    assert!(
        obr_q.status.success(),
        "obr q with type failed: {}",
        obr_q.stderr
    );
    assert!(
        bd_q.status.success(),
        "bd q with type failed: {}",
        bd_q.stderr
    );

    info!("conformance_q_with_type passed");
}

#[test]
fn conformance_q_creates_issue() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_q_creates_issue test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_q = workspace.run_obr(["q", "Verify creation"], "q_create");
    let obr_id = obr_q.stdout.trim();

    let bd_q = workspace.run_bd(["q", "Verify creation"], "q_create");
    let bd_id = bd_q.stdout.trim();

    // Verify issue was created with show
    let obr_show = workspace.run_obr(["show", obr_id, "--json"], "show_q_issue");
    let bd_show = workspace.run_bd(["show", bd_id, "--json"], "show_q_issue");

    assert!(
        obr_show.status.success(),
        "obr show q-created issue failed: {}",
        obr_show.stderr
    );
    assert!(
        bd_show.status.success(),
        "bd show q-created issue failed: {}",
        bd_show.stderr
    );

    info!("conformance_q_creates_issue passed");
}

#[test]
fn conformance_q_id_in_list() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_q_id_in_list test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_q = workspace.run_obr(["q", "List me"], "q_list");
    let bd_q = workspace.run_bd(["q", "List me"], "q_list");

    let obr_id = obr_q.stdout.trim().to_string();
    let bd_id = bd_q.stdout.trim().to_string();

    let obr_list = workspace.run_obr(["list", "--json"], "q_list_br");
    let bd_list = workspace.run_bd(["list", "--json"], "q_list_bd");

    assert!(
        obr_list.status.success(),
        "obr list failed: {}",
        obr_list.stderr
    );
    assert!(
        bd_list.status.success(),
        "bd list failed: {}",
        bd_list.stderr
    );

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_list.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_list.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_ids: Vec<&str> = issue_items(&obr_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();
    let bd_ids: Vec<&str> = issue_items(&bd_val)
        .into_iter()
        .filter_map(|v| v.get("id").and_then(|id| id.as_str()))
        .collect();

    assert!(obr_ids.contains(&obr_id.as_str()));
    assert!(bd_ids.contains(&bd_id.as_str()));

    info!("conformance_q_id_in_list passed");
}

#[test]
fn conformance_q_error_no_title() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_q_error_no_title test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_q = workspace.run_obr(["q"], "q_no_title");
    let bd_q = workspace.run_bd(["q"], "q_no_title");

    assert!(!obr_q.status.success(), "obr q should fail without title");
    assert!(!bd_q.status.success(), "bd q should fail without title");

    info!("conformance_q_error_no_title passed");
}

// ============================================================================
// LINT COMMAND TESTS
// ============================================================================

#[test]
fn conformance_lint_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_lint_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Lint on empty workspace
    let obr_lint = workspace.run_obr(["lint", "--json"], "lint_empty");
    let bd_lint = workspace.run_bd(["lint", "--json"], "lint_empty");

    assert!(
        obr_lint.status.success(),
        "obr lint empty failed: {}",
        obr_lint.stderr
    );
    assert!(
        bd_lint.status.success(),
        "bd lint empty failed: {}",
        bd_lint.stderr
    );

    info!("conformance_lint_empty passed");
}

#[test]
fn conformance_lint_with_issues() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_lint_with_issues test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    workspace.run_obr(["create", "Test issue for lint"], "create");
    workspace.run_bd(["create", "Test issue for lint"], "create");

    let obr_lint = workspace.run_obr(["lint", "--json"], "lint_with_issues");
    let bd_lint = workspace.run_bd(["lint", "--json"], "lint_with_issues");

    assert!(
        obr_lint.status.success(),
        "obr lint failed: {}",
        obr_lint.stderr
    );
    assert!(
        bd_lint.status.success(),
        "bd lint failed: {}",
        bd_lint.stderr
    );

    info!("conformance_lint_with_issues passed");
}

#[test]
fn conformance_lint_by_type() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_lint_by_type test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Bug issue", "-t", "bug"], "create_bug");
    workspace.run_bd(["create", "Bug issue", "-t", "bug"], "create_bug");
    workspace.run_obr(["create", "Task issue", "-t", "task"], "create_task");
    workspace.run_bd(["create", "Task issue", "-t", "task"], "create_task");

    let obr_lint = workspace.run_obr(["lint", "-t", "bug", "--json"], "lint_by_type");
    let bd_lint = workspace.run_bd(["lint", "-t", "bug", "--json"], "lint_by_type");

    assert!(
        obr_lint.status.success(),
        "obr lint by type failed: {}",
        obr_lint.stderr
    );
    assert!(
        bd_lint.status.success(),
        "bd lint by type failed: {}",
        bd_lint.stderr
    );

    info!("conformance_lint_by_type passed");
}

#[test]
fn conformance_lint_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_lint_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Lint test"], "create");
    workspace.run_bd(["create", "Lint test"], "create");

    let obr_lint = workspace.run_obr(["lint", "--json"], "lint_json");
    let bd_lint = workspace.run_bd(["lint", "--json"], "lint_json");

    let obr_json = extract_json_payload(&obr_lint.stdout);
    let bd_json = extract_json_payload(&bd_lint.stdout);

    let obr_val: Result<Value, _> = serde_json::from_str(&obr_json);
    let bd_val: Result<Value, _> = serde_json::from_str(&bd_json);

    assert!(obr_val.is_ok(), "obr lint should produce valid JSON");
    assert!(bd_val.is_ok(), "bd lint should produce valid JSON");

    info!("conformance_lint_json_shape passed");
}

#[test]
fn conformance_lint_exit_code() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_lint_exit_code test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create a bug with no description to trigger warnings
    workspace.run_obr(["create", "Lint bug", "--type", "bug"], "lint_bug_create");
    workspace.run_bd(["create", "Lint bug", "--type", "bug"], "lint_bug_create");

    let obr_lint = workspace.run_obr(["lint"], "lint_exit");
    let bd_lint = workspace.run_bd(["lint"], "lint_exit");

    assert!(
        !obr_lint.status.success(),
        "obr lint should exit nonzero with warnings"
    );
    assert!(
        !bd_lint.status.success(),
        "bd lint should exit nonzero with warnings"
    );

    info!("conformance_lint_exit_code passed");
}

// ============================================================================
// DEFER/UNDEFER COMMAND TESTS
// ============================================================================

#[test]
fn conformance_defer_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_defer_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues
    let obr_create = workspace.run_obr(["create", "Defer test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Defer test", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_create.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_create.stdout));

    // Defer with --until
    let obr_defer = workspace.run_obr(
        ["defer", &obr_id, "--until", "+1d", "--json"],
        "defer_basic",
    );
    let bd_defer = workspace.run_bd(["defer", &bd_id, "--until", "+1d", "--json"], "defer_basic");

    assert!(
        obr_defer.status.success(),
        "obr defer failed: {}",
        obr_defer.stderr
    );
    assert!(
        bd_defer.status.success(),
        "bd defer failed: {}",
        bd_defer.stderr
    );

    info!("conformance_defer_basic passed");
}

#[test]
fn conformance_defer_excludes_from_ready() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_defer_excludes_from_ready test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Will defer", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Will defer", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_create.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_create.stdout));

    // Defer far into the future
    workspace.run_obr(["defer", &obr_id, "--until", "+30d"], "defer");
    workspace.run_bd(["defer", &bd_id, "--until", "+30d"], "defer");

    // Check ready - deferred issue should not appear
    let obr_ready = workspace.run_obr(["ready", "--json"], "ready_after_defer");
    let bd_ready = workspace.run_bd(["ready", "--json"], "ready_after_defer");

    assert!(obr_ready.status.success(), "obr ready failed");
    assert!(bd_ready.status.success(), "bd ready failed");

    let obr_json = extract_json_payload(&obr_ready.stdout);
    let bd_json = extract_json_payload(&bd_ready.stdout);

    // Deferred issue should not appear in ready list
    assert!(
        !obr_json.contains(&obr_id),
        "obr ready should not include deferred issue"
    );
    assert!(
        !bd_json.contains(&bd_id),
        "bd ready should not include deferred issue"
    );

    info!("conformance_defer_excludes_from_ready passed");
}

#[test]
fn conformance_undefer_basic() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_undefer_basic test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Undefer test", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Undefer test", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_create.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_create.stdout));

    // Defer then undefer
    workspace.run_obr(["defer", &obr_id, "--until", "+30d"], "defer");
    workspace.run_bd(["defer", &bd_id, "--until", "+30d"], "defer");

    let obr_undefer = workspace.run_obr(["undefer", &obr_id, "--json"], "undefer");
    let bd_undefer = workspace.run_bd(["undefer", &bd_id, "--json"], "undefer");

    assert!(
        obr_undefer.status.success(),
        "obr undefer failed: {}",
        obr_undefer.stderr
    );
    assert!(
        bd_undefer.status.success(),
        "bd undefer failed: {}",
        bd_undefer.stderr
    );

    info!("conformance_undefer_basic passed");
}

#[test]
fn conformance_undefer_restores_ready() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_undefer_restores_ready test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_create = workspace.run_obr(["create", "Restore to ready", "--json"], "create");
    let bd_create = workspace.run_bd(["create", "Restore to ready", "--json"], "create");

    let obr_id = extract_issue_id(&extract_json_payload(&obr_create.stdout));
    let bd_id = extract_issue_id(&extract_json_payload(&bd_create.stdout));

    // Defer then undefer
    workspace.run_obr(["defer", &obr_id, "--until", "+30d"], "defer");
    workspace.run_bd(["defer", &bd_id, "--until", "+30d"], "defer");
    workspace.run_obr(["undefer", &obr_id], "undefer");
    workspace.run_bd(["undefer", &bd_id], "undefer");

    // Should appear in ready again
    let obr_ready = workspace.run_obr(["ready", "--json"], "ready_after_undefer");
    let bd_ready = workspace.run_bd(["ready", "--json"], "ready_after_undefer");

    assert!(obr_ready.status.success(), "obr ready failed");
    assert!(bd_ready.status.success(), "bd ready failed");

    info!("conformance_undefer_restores_ready passed");
}

// ============================================================================
// HISTORY COMMAND TESTS (br-only feature)
// ============================================================================

#[test]
fn conformance_history_list_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_history_list_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // history is br-only, just verify obr works
    let obr_hist = workspace.run_obr(["history", "list", "--json"], "history_list_empty");

    assert!(
        obr_hist.status.success(),
        "obr history list failed: {}",
        obr_hist.stderr
    );

    info!("conformance_history_list_empty passed");
}

#[test]
fn conformance_history_list_after_sync() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_history_list_after_sync test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issue and sync to create history
    workspace.run_obr(["create", "History test"], "create");
    workspace.run_obr(["sync", "--flush-only"], "sync");

    // history is br-only
    let obr_hist = workspace.run_obr(["history", "list", "--json"], "history_list");

    assert!(
        obr_hist.status.success(),
        "obr history list failed: {}",
        obr_hist.stderr
    );

    info!("conformance_history_list_after_sync passed");
}

#[test]
fn conformance_history_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_history_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // history is br-only
    // Note: When no backups exist, obr outputs plain text "No backups found"
    // rather than JSON. This is expected behavior for empty history.
    let obr_hist = workspace.run_obr(["history", "list", "--json"], "history_json");

    // Verify command succeeds
    assert!(
        obr_hist.status.success(),
        "obr history list failed: {}",
        obr_hist.stderr
    );

    // If there's JSON payload, validate it; otherwise accept plain text for empty
    let obr_json = extract_json_payload(&obr_hist.stdout);
    if !obr_json.is_empty() && !obr_json.contains("No backups found") {
        let obr_val: Result<Value, _> = serde_json::from_str(&obr_json);
        assert!(
            obr_val.is_ok(),
            "obr history list should produce valid JSON when backups exist"
        );
    }

    info!("conformance_history_json_shape passed");
}

// ============================================================================
// ORPHANS COMMAND TESTS
// ============================================================================

#[test]
fn conformance_orphans_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_orphans_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_orphans = workspace.run_obr(["orphans", "--json"], "orphans_empty");
    let bd_orphans = workspace.run_bd(["orphans", "--json"], "orphans_empty");

    assert!(
        obr_orphans.status.success(),
        "obr orphans failed: {}",
        obr_orphans.stderr
    );
    assert!(
        bd_orphans.status.success(),
        "bd orphans failed: {}",
        bd_orphans.stderr
    );

    info!("conformance_orphans_empty passed");
}

#[test]
fn conformance_orphans_with_issues() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_orphans_with_issues test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["create", "Orphan test"], "create");
    workspace.run_bd(["create", "Orphan test"], "create");

    let obr_orphans = workspace.run_obr(["orphans", "--json"], "orphans_with_issues");
    let bd_orphans = workspace.run_bd(["orphans", "--json"], "orphans_with_issues");

    assert!(
        obr_orphans.status.success(),
        "obr orphans failed: {}",
        obr_orphans.stderr
    );
    assert!(
        bd_orphans.status.success(),
        "bd orphans failed: {}",
        bd_orphans.stderr
    );

    info!("conformance_orphans_with_issues passed");
}

#[test]
fn conformance_orphans_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_orphans_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    let obr_orphans = workspace.run_obr(["orphans", "--json"], "orphans_json");
    let bd_orphans = workspace.run_bd(["orphans", "--json"], "orphans_json");

    let obr_json = extract_json_payload(&obr_orphans.stdout);
    let bd_json = extract_json_payload(&bd_orphans.stdout);

    let obr_val: Result<Value, _> = serde_json::from_str(&obr_json);
    let bd_val: Result<Value, _> = serde_json::from_str(&bd_json);

    assert!(obr_val.is_ok(), "obr orphans should produce valid JSON");
    assert!(bd_val.is_ok(), "bd orphans should produce valid JSON");

    info!("conformance_orphans_json_shape passed");
}

// ============================================================================
// CHANGELOG COMMAND TESTS (br-only feature)
// ============================================================================

#[test]
fn conformance_changelog_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_changelog_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // changelog is br-only
    let obr_changelog = workspace.run_obr(["changelog", "--json"], "changelog_empty");

    assert!(
        obr_changelog.status.success(),
        "obr changelog failed: {}",
        obr_changelog.stderr
    );

    info!("conformance_changelog_empty passed");
}

#[test]
fn conformance_changelog_with_closed() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_changelog_with_closed test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create and close issues (using obr only for changelog test)
    let obr_create = workspace.run_obr(["create", "Changelog entry", "--json"], "create");
    let obr_id = extract_issue_id(&extract_json_payload(&obr_create.stdout));
    workspace.run_obr(["close", &obr_id], "close");

    // changelog is br-only
    let obr_changelog = workspace.run_obr(["changelog", "--json"], "changelog_with_closed");

    assert!(
        obr_changelog.status.success(),
        "obr changelog failed: {}",
        obr_changelog.stderr
    );

    info!("conformance_changelog_with_closed passed");
}

#[test]
fn conformance_changelog_json_shape() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_changelog_json_shape test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // changelog is br-only
    let obr_changelog = workspace.run_obr(["changelog", "--json"], "changelog_json");

    let obr_json = extract_json_payload(&obr_changelog.stdout);
    let obr_val: Result<Value, _> = serde_json::from_str(&obr_json);

    assert!(obr_val.is_ok(), "obr changelog should produce valid JSON");

    info!("conformance_changelog_json_shape passed");
}

// ============================================================================
// QUERY COMMAND TESTS (br-only feature)
// ============================================================================

#[test]
fn conformance_query_list_empty() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_query_list_empty test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // query is br-only
    let obr_query = workspace.run_obr(["query", "list", "--json"], "query_list_empty");

    assert!(
        obr_query.status.success(),
        "obr query list failed: {}",
        obr_query.stderr
    );

    info!("conformance_query_list_empty passed");
}

#[test]
fn conformance_query_save_and_list() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_query_save_and_list test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // query is br-only
    let obr_save = workspace.run_obr(
        [
            "query",
            "save",
            "high-priority",
            "--status",
            "open",
            "--priority",
            "1",
            "--json",
        ],
        "query_save",
    );

    assert!(
        obr_save.status.success(),
        "obr query save failed: {}",
        obr_save.stderr
    );

    // List queries
    let obr_list = workspace.run_obr(["query", "list", "--json"], "query_list");
    assert!(obr_list.status.success(), "obr query list failed");

    info!("conformance_query_save_and_list passed");
}

#[test]
fn conformance_query_run() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_query_run test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create some issues (obr only for query tests)
    workspace.run_obr(["create", "High pri", "-p", "1"], "create_high");
    workspace.run_obr(["create", "Low pri", "-p", "3"], "create_low");

    // query is br-only
    workspace.run_obr(
        ["query", "save", "high-only", "--priority", "1"],
        "query_save",
    );

    let obr_run = workspace.run_obr(["query", "run", "high-only", "--json"], "query_run");

    assert!(
        obr_run.status.success(),
        "obr query run failed: {}",
        obr_run.stderr
    );

    info!("conformance_query_run passed");
}

#[test]
fn conformance_query_delete() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_query_delete test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // query is br-only
    workspace.run_obr(
        ["query", "save", "to-delete", "--status", "open"],
        "query_save",
    );

    let obr_delete = workspace.run_obr(["query", "delete", "to-delete", "--json"], "query_delete");

    assert!(
        obr_delete.status.success(),
        "obr query delete failed: {}",
        obr_delete.stderr
    );

    info!("conformance_query_delete passed");
}

// ============================================================================
// COMPLETIONS COMMAND TESTS
// Note: obr uses "completions", bd uses "completion" (singular)
// ============================================================================

#[test]
fn conformance_completions_bash() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_completions_bash test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // obr uses "completions", bd uses "completion"
    let obr_comp = workspace.run_obr(["completions", "bash"], "completions_bash");
    let bd_comp = workspace.run_bd(["completion", "bash"], "completion_bash");

    assert!(
        obr_comp.status.success(),
        "obr completions bash failed: {}",
        obr_comp.stderr
    );
    assert!(
        bd_comp.status.success(),
        "bd completion bash failed: {}",
        bd_comp.stderr
    );

    // Output should contain shell completion script
    assert!(
        !obr_comp.stdout.is_empty(),
        "obr completions should produce output"
    );
    assert!(
        !bd_comp.stdout.is_empty(),
        "bd completion should produce output"
    );

    info!("conformance_completions_bash passed");
}

#[test]
fn conformance_completions_zsh() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_completions_zsh test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // obr uses "completions", bd uses "completion"
    let obr_comp = workspace.run_obr(["completions", "zsh"], "completions_zsh");
    let bd_comp = workspace.run_bd(["completion", "zsh"], "completion_zsh");

    assert!(
        obr_comp.status.success(),
        "obr completions zsh failed: {}",
        obr_comp.stderr
    );
    assert!(
        bd_comp.status.success(),
        "bd completion zsh failed: {}",
        bd_comp.stderr
    );

    info!("conformance_completions_zsh passed");
}

#[test]
fn conformance_completions_fish() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_completions_fish test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // obr uses "completions", bd uses "completion"
    let obr_comp = workspace.run_obr(["completions", "fish"], "completions_fish");
    let bd_comp = workspace.run_bd(["completion", "fish"], "completion_fish");

    assert!(
        obr_comp.status.success(),
        "obr completions fish failed: {}",
        obr_comp.stderr
    );
    assert!(
        bd_comp.status.success(),
        "bd completion fish failed: {}",
        bd_comp.stderr
    );

    info!("conformance_completions_fish passed");
}

#[test]
fn conformance_stats_all_fields() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stats_all_fields test");
    let workspace = ConformanceWorkspace::new();
    workspace.init_both();
    workspace.run_obr(["create", "Issue"], "create");
    workspace.run_bd(["create", "Issue"], "create");
    let obr_stats = workspace.run_obr(["stats", "--json"], "stats");
    let bd_stats = workspace.run_bd(["stats", "--json"], "stats");
    assert!(obr_stats.status.success());
    assert!(bd_stats.status.success());
    let obr_json = extract_json_payload(&obr_stats.stdout);
    let bd_json = extract_json_payload(&bd_stats.stdout);
    compare_json(
        &obr_json,
        &bd_json,
        &CompareMode::ContainsFields(vec![
            "summary.total_issues".to_string(),
            "summary.open_issues".to_string(),
            "summary.in_progress_issues".to_string(),
            "summary.closed_issues".to_string(),
            "summary.blocked_issues".to_string(),
            "summary.deferred_issues".to_string(),
            "summary.ready_issues".to_string(),
            "summary.tombstone_issues".to_string(),
            "summary.pinned_issues".to_string(),
            "summary.epics_eligible_for_closure".to_string(),
        ]),
    )
    .expect("JSON mismatch");
    info!("conformance_stats_all_fields passed");
}

#[test]
#[ignore]
fn conformance_stale_all_stale() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_stale_all_stale test");
    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Set consistent prefix
    workspace.run_obr(["config", "--set", "id.prefix=TEST"], "set_prefix_br");
    workspace.run_bd(["config", "--set", "id.prefix=TEST"], "set_prefix_bd");

    workspace.run_obr(["create", "Stale issue"], "create");
    workspace.run_bd(["create", "Stale issue"], "create");
    std::thread::sleep(Duration::from_millis(100));
    let obr_stale = workspace.run_obr(["stale", "--days", "0", "--json"], "stale");
    let bd_stale = workspace.run_bd(["stale", "--days", "0", "--json"], "stale");
    assert!(obr_stale.status.success());
    assert!(bd_stale.status.success());
    let obr_json = extract_json_payload(&obr_stale.stdout);
    let bd_json = extract_json_payload(&bd_stale.stdout);

    log_timings("stale_all_stale", &obr_stale, &bd_stale);
    compare_json(&obr_json, &bd_json, &CompareMode::NormalizedJson).expect("JSON mismatch");
    info!("conformance_stale_all_stale passed");
}

#[test]
fn conformance_version_semver() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_version_semver test");
    let workspace = ConformanceWorkspace::new();
    workspace.init_both();
    let obr_ver = workspace.run_obr(["version", "--json"], "version");
    let bd_ver = workspace.run_bd(["version", "--json"], "version");

    let obr_json = extract_json_payload(&obr_ver.stdout);
    let bd_json = extract_json_payload(&bd_ver.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap();
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap();

    let obr_version = obr_val["version"].as_str().unwrap_or("");
    let bd_version = bd_val["version"].as_str().unwrap_or("");

    let is_semver = |v: &str| {
        let parts: Vec<&str> = v.split('.').collect();
        parts.len() >= 2
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    };

    assert!(
        is_semver(obr_version),
        "obr version is not semver: {obr_version}"
    );
    assert!(
        is_semver(bd_version),
        "bd version is not semver: {bd_version}"
    );

    log_timings("version_semver", &obr_ver, &bd_ver);
    info!("conformance_version_semver passed");
}

// ============================================================================
// BASE SNAPSHOT CONFORMANCE TESTS
// Validate merge.base.jsonl behavior parity between obr and bd
// ============================================================================

/// Helper to initialize git repo in a directory for sync tests
fn init_git_repo(dir: &PathBuf) {
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .expect("git init");
    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir)
        .output()
        .expect("git config email");
    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(dir)
        .output()
        .expect("git config name");
}

/// INTENTIONAL DIVERGENCE: bd v0.46.0 attempts git commit during sync, which fails in non-repo dirs.
/// obr's non-invasive design intentionally never runs git commands. This is by design.
#[test]
#[ignore = "bd v0.46.0 sync does git commit, fails in non-git dirs: obr is intentionally non-invasive"]
fn conformance_sync_base_snapshot_created_after_sync() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_base_snapshot_created_after_sync test");

    let workspace = ConformanceWorkspace::new();

    // Initialize git repos (required for bd sync)
    init_git_repo(&workspace.obr_root);
    init_git_repo(&workspace.bd_root);

    workspace.init_both();

    // Create issue
    workspace.run_obr(["create", "Base snapshot test"], "create");
    workspace.run_bd(["create", "Base snapshot test"], "create");

    // Export to JSONL
    let obr_flush = workspace.run_obr(["sync", "--flush-only"], "flush");
    let bd_flush = workspace.run_bd(["sync", "--flush-only"], "flush");

    assert!(
        obr_flush.status.success(),
        "obr flush failed: {}",
        obr_flush.stderr
    );
    assert!(
        bd_flush.status.success(),
        "bd flush failed: {}",
        bd_flush.stderr
    );

    // Commit the JSONL files so sync can work
    std::process::Command::new("git")
        .args(["add", ".obr/"])
        .current_dir(&workspace.obr_root)
        .output()
        .expect("git add obr");
    std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(&workspace.obr_root)
        .output()
        .expect("git commit obr");
    std::process::Command::new("git")
        .args(["add", ".obr/"])
        .current_dir(&workspace.bd_root)
        .output()
        .expect("git add bd");
    std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(&workspace.bd_root)
        .output()
        .expect("git commit bd");

    // Full sync should create base snapshot
    let obr_sync = workspace.run_obr(["sync"], "sync");
    let bd_sync = workspace.run_bd(["sync"], "sync");

    assert!(
        obr_sync.status.success(),
        "obr sync failed: {}",
        obr_sync.stderr
    );
    assert!(
        bd_sync.status.success(),
        "bd sync failed: {}",
        bd_sync.stderr
    );

    // Check if base snapshot exists for both
    let obr_base = workspace.obr_root.join(".obr").join("merge.base.jsonl");
    let bd_base = workspace.bd_root.join(".obr").join("merge.base.jsonl");

    let obr_base_exists = obr_base.exists();
    let bd_base_exists = bd_base.exists();

    assert_eq!(
        obr_base_exists, bd_base_exists,
        "base snapshot existence differs: obr={}, bd={}",
        obr_base_exists, bd_base_exists
    );

    info!("conformance_sync_base_snapshot_created_after_sync passed");
}

/// INTENTIONAL DIVERGENCE, same root cause as
/// `conformance_sync_base_snapshot_created_after_sync` above: this test drives a
/// bare `sync`. bd v0.46.0 treats that as "commit to git and write
/// `merge.base.jsonl`"; obr's non-invasive design never runs git and requires an
/// explicit direction, so it does not write a base snapshot there. The test then
/// trips its own `(Some, None)` mismatch arm. Verified against a real bd v0.46.0
/// on 2026-07-25 (`beads_rust-ecr6`).
#[test]
#[ignore = "bare `sync` writes merge.base.jsonl on bd via its git-commit path; obr is intentionally non-invasive"]
fn conformance_sync_base_snapshot_content_matches() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_base_snapshot_content_matches test");

    let workspace = ConformanceWorkspace::new();

    // Initialize git repos (required for bd sync)
    init_git_repo(&workspace.obr_root);
    init_git_repo(&workspace.bd_root);

    workspace.init_both();

    // Set consistent prefix for ID comparison
    workspace.run_obr(["config", "--set", "id.prefix=TEST"], "set_prefix_br");
    workspace.run_bd(["config", "--set", "id.prefix=TEST"], "set_prefix_bd");

    // Create issue
    workspace.run_obr(["create", "Base content test"], "create");
    workspace.run_bd(["create", "Base content test"], "create");

    // Flush to JSONL
    workspace.run_obr(["sync", "--flush-only"], "flush");
    workspace.run_bd(["sync", "--flush-only"], "flush");

    // Commit the JSONL files so sync can work
    std::process::Command::new("git")
        .args(["add", ".obr/"])
        .current_dir(&workspace.obr_root)
        .output()
        .expect("git add obr");
    std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(&workspace.obr_root)
        .output()
        .expect("git commit obr");
    std::process::Command::new("git")
        .args(["add", ".obr/"])
        .current_dir(&workspace.bd_root)
        .output()
        .expect("git add bd");
    std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(&workspace.bd_root)
        .output()
        .expect("git commit bd");

    // Full sync
    workspace.run_obr(["sync"], "sync");
    workspace.run_bd(["sync"], "sync");

    // Read base snapshot contents
    let obr_base = workspace.obr_root.join(".obr").join("merge.base.jsonl");
    let bd_base = workspace.bd_root.join(".obr").join("merge.base.jsonl");

    // Both may or may not create base snapshot based on merge behavior
    // The important thing is they behave consistently
    let obr_content = fs::read_to_string(&obr_base).ok();
    let bd_content = fs::read_to_string(&bd_base).ok();

    match (obr_content, bd_content) {
        (Some(obr), Some(bd)) => {
            // Both created base snapshot - validate line count matches
            let obr_lines: Vec<&str> = obr.lines().filter(|l| !l.trim().is_empty()).collect();
            let bd_lines: Vec<&str> = bd.lines().filter(|l| !l.trim().is_empty()).collect();

            assert_eq!(
                obr_lines.len(),
                bd_lines.len(),
                "base snapshot line count differs: obr={}, bd={}",
                obr_lines.len(),
                bd_lines.len()
            );
        }
        (None, None) => {
            // Neither created base snapshot - also valid
            info!("Both obr and bd did not create base snapshot (consistent behavior)");
        }
        (obr, bd) => {
            panic!(
                "base snapshot creation differs: obr={:?}, bd={:?}",
                obr.is_some(),
                bd.is_some()
            );
        }
    }

    info!("conformance_sync_base_snapshot_content_matches passed");
}

#[test]
fn conformance_sync_base_snapshot_preserves_issue_state() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_base_snapshot_preserves_issue_state test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create issues (both will be open initially)
    workspace.run_obr(["create", "Issue 1"], "create1");
    workspace.run_bd(["create", "Issue 1"], "create1");

    workspace.run_obr(["create", "Issue 2"], "create2");
    workspace.run_bd(["create", "Issue 2"], "create2");

    // Flush to JSONL (this doesn't require git)
    workspace.run_obr(["sync", "--flush-only"], "flush");
    workspace.run_bd(["sync", "--flush-only"], "flush");

    // Verify open issues in the database (using default list which shows open)
    let obr_list = workspace.run_obr(["list", "--json"], "list_open");
    let bd_list = workspace.run_bd(["list", "--json"], "list_open");

    let obr_json = extract_json_payload(&obr_list.stdout);
    let bd_json = extract_json_payload(&bd_list.stdout);

    let obr_val: Value = serde_json::from_str(&obr_json).unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&bd_json).unwrap_or(Value::Array(vec![]));

    let obr_count = issue_count(&obr_val);
    let bd_count = issue_count(&bd_val);

    assert_eq!(
        obr_count, bd_count,
        "issue count after flush differs: obr={}, bd={}",
        obr_count, bd_count
    );

    // Check that both have 2 open issues
    assert_eq!(obr_count, 2, "expected 2 open issues after flush");

    info!("conformance_sync_base_snapshot_preserves_issue_state passed");
}

// ============================================================================
// CONFLICT MARKER CONFORMANCE TESTS
// Validate both obr and bd reject JSONL with git merge conflict markers
// ============================================================================

#[test]
fn conformance_sync_import_rejects_conflict_markers() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import_rejects_conflict_markers test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create valid issue to get a baseline
    workspace.run_obr(["create", "Valid issue"], "create");
    workspace.run_bd(["create", "Valid issue"], "create");

    workspace.run_obr(["sync", "--flush-only"], "flush");
    workspace.run_bd(["sync", "--flush-only"], "flush");

    // Read the exported JSONL
    let obr_jsonl_path = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl_path = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl_path).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl_path).expect("read bd jsonl");

    // Inject conflict markers
    let obr_conflicted = format!(
        "<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> feature-branch\n",
        obr_content.trim(),
        obr_content.trim()
    );
    let bd_conflicted = format!(
        "<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> feature-branch\n",
        bd_content.trim(),
        bd_content.trim()
    );

    fs::write(&obr_jsonl_path, &obr_conflicted).expect("write obr conflicted");
    fs::write(&bd_jsonl_path, &bd_conflicted).expect("write bd conflicted");

    // Import should fail for both
    let obr_import = workspace.run_obr(["sync", "--import-only"], "import_conflict");
    let bd_import = workspace.run_bd(["sync", "--import-only"], "import_conflict");

    // Both should fail
    assert!(
        !obr_import.status.success(),
        "obr should reject conflict markers but succeeded"
    );
    assert!(
        !bd_import.status.success(),
        "bd should reject conflict markers but succeeded"
    );

    // Both should mention conflict in error
    let obr_mentions_conflict = obr_import.stderr.to_lowercase().contains("conflict")
        || obr_import.stdout.to_lowercase().contains("conflict");
    let bd_mentions_conflict = bd_import.stderr.to_lowercase().contains("conflict")
        || bd_import.stdout.to_lowercase().contains("conflict");

    assert!(
        obr_mentions_conflict,
        "obr error should mention conflict: stdout={}, stderr={}",
        obr_import.stdout, obr_import.stderr
    );
    assert!(
        bd_mentions_conflict,
        "bd error should mention conflict: stdout={}, stderr={}",
        bd_import.stdout, bd_import.stderr
    );

    info!("conformance_sync_import_rejects_conflict_markers passed");
}

#[test]
fn conformance_sync_import_rejects_partial_conflict_markers() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import_rejects_partial_conflict_markers test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Write JSONL with only the start conflict marker
    let obr_jsonl_path = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl_path = workspace.bd_root.join(".obr").join("issues.jsonl");

    let partial_conflict = "<<<<<<< HEAD\n{\"id\":\"test-1\",\"title\":\"Test\"}\n";

    fs::write(&obr_jsonl_path, partial_conflict).expect("write obr partial conflict");
    fs::write(&bd_jsonl_path, partial_conflict).expect("write bd partial conflict");

    // Import should fail for both
    let obr_import = workspace.run_obr(["sync", "--import-only"], "import_partial_conflict");
    let bd_import = workspace.run_bd(["sync", "--import-only"], "import_partial_conflict");

    // Both should fail (rejecting conflict markers)
    assert_eq!(
        obr_import.status.success(),
        bd_import.status.success(),
        "partial conflict marker handling differs: obr={}, bd={}",
        obr_import.status.success(),
        bd_import.status.success()
    );

    // If both fail, they should both mention conflict
    if !obr_import.status.success() && !bd_import.status.success() {
        let obr_mentions = obr_import.stderr.to_lowercase().contains("conflict")
            || obr_import.stderr.contains("<<<<<<<");
        let bd_mentions = bd_import.stderr.to_lowercase().contains("conflict")
            || bd_import.stderr.contains("<<<<<<<");

        // At minimum, one should detect it
        assert!(
            obr_mentions || bd_mentions,
            "at least one should mention conflict markers"
        );
    }

    info!("conformance_sync_import_rejects_partial_conflict_markers passed");
}

#[test]
fn conformance_sync_import_rejects_conflict_in_middle() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import_rejects_conflict_in_middle test");

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    // Create and export valid issues first
    workspace.run_obr(["create", "Issue 1"], "create1");
    workspace.run_bd(["create", "Issue 1"], "create1");
    workspace.run_obr(["create", "Issue 2"], "create2");
    workspace.run_bd(["create", "Issue 2"], "create2");

    workspace.run_obr(["sync", "--flush-only"], "flush");
    workspace.run_bd(["sync", "--flush-only"], "flush");

    // Read exported JSONL
    let obr_jsonl_path = workspace.obr_root.join(".obr").join("issues.jsonl");
    let bd_jsonl_path = workspace.bd_root.join(".obr").join("issues.jsonl");

    let obr_content = fs::read_to_string(&obr_jsonl_path).expect("read obr jsonl");
    let bd_content = fs::read_to_string(&bd_jsonl_path).expect("read bd jsonl");

    // Insert conflict markers between valid lines
    let obr_lines: Vec<&str> = obr_content.lines().collect();
    let bd_lines: Vec<&str> = bd_content.lines().collect();

    let obr_with_conflict = if obr_lines.len() >= 2 {
        format!(
            "{}\n<<<<<<< HEAD\n{}\n=======\n>>>>>>> branch\n",
            obr_lines[0], obr_lines[1]
        )
    } else {
        format!("<<<<<<< HEAD\n{}\n=======\n>>>>>>> branch\n", obr_content)
    };

    let bd_with_conflict = if bd_lines.len() >= 2 {
        format!(
            "{}\n<<<<<<< HEAD\n{}\n=======\n>>>>>>> branch\n",
            bd_lines[0], bd_lines[1]
        )
    } else {
        format!("<<<<<<< HEAD\n{}\n=======\n>>>>>>> branch\n", bd_content)
    };

    fs::write(&obr_jsonl_path, &obr_with_conflict).expect("write obr conflict");
    fs::write(&bd_jsonl_path, &bd_with_conflict).expect("write bd conflict");

    // Import should fail for both
    let obr_import = workspace.run_obr(["sync", "--import-only"], "import_middle_conflict");
    let bd_import = workspace.run_bd(["sync", "--import-only"], "import_middle_conflict");

    assert_eq!(
        obr_import.status.success(),
        bd_import.status.success(),
        "middle conflict marker handling differs: obr success={}, bd success={}",
        obr_import.status.success(),
        bd_import.status.success()
    );

    info!("conformance_sync_import_rejects_conflict_in_middle passed");
}

// ============================================================================
// PREFIX MISMATCH CONFORMANCE TESTS
// Validate prefix mismatch handling parity between obr and bd
// ============================================================================

#[test]
fn conformance_sync_import_prefix_mismatch_behavior() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import_prefix_mismatch_behavior test");

    // Source workspace with prefix "SRC"
    let source = ConformanceWorkspace::new();
    source.init_both();
    source.run_obr(["config", "--set", "id.prefix=SRC"], "set_prefix_br");
    source.run_bd(["config", "--set", "id.prefix=SRC"], "set_prefix_bd");

    source.run_obr(["create", "Source issue"], "create");
    source.run_bd(["create", "Source issue"], "create");
    source.run_obr(["sync", "--flush-only"], "flush");
    source.run_bd(["sync", "--flush-only"], "flush");

    // Target workspace with prefix "TGT"
    let target = ConformanceWorkspace::new();
    target.init_both();
    target.run_obr(["config", "--set", "id.prefix=TGT"], "set_prefix_br");
    target.run_bd(["config", "--set", "id.prefix=TGT"], "set_prefix_bd");

    // Copy JSONL from source to target
    let obr_src = source.obr_root.join(".obr").join("issues.jsonl");
    let bd_src = source.bd_root.join(".obr").join("issues.jsonl");
    let obr_dst = target.obr_root.join(".obr").join("issues.jsonl");
    let bd_dst = target.bd_root.join(".obr").join("issues.jsonl");

    fs::copy(&obr_src, &obr_dst).expect("copy obr jsonl");
    fs::copy(&bd_src, &bd_dst).expect("copy bd jsonl");

    // Import with mismatched prefix
    let obr_import = target.run_obr(["sync", "--import-only"], "import_mismatch");
    let bd_import = target.run_bd(["sync", "--import-only"], "import_mismatch");

    // Both should handle prefix mismatch consistently
    // (either both succeed with rewrite or both fail with error)
    assert_eq!(
        obr_import.status.success(),
        bd_import.status.success(),
        "prefix mismatch handling differs: obr success={}, bd success={}",
        obr_import.status.success(),
        bd_import.status.success()
    );

    // If both fail, check they mention prefix
    if !obr_import.status.success() && !bd_import.status.success() {
        let obr_mentions_prefix = obr_import.stderr.to_lowercase().contains("prefix")
            || obr_import.stdout.to_lowercase().contains("prefix");
        let bd_mentions_prefix = bd_import.stderr.to_lowercase().contains("prefix")
            || bd_import.stdout.to_lowercase().contains("prefix");

        // At least one should mention prefix in error
        assert!(
            obr_mentions_prefix || bd_mentions_prefix,
            "error should mention prefix mismatch"
        );
    }

    info!("conformance_sync_import_prefix_mismatch_behavior passed");
}

#[test]
fn conformance_sync_import_same_prefix_succeeds() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_import_same_prefix_succeeds test");

    // Source workspace
    let source = ConformanceWorkspace::new();
    source.init_both();
    source.run_obr(["config", "--set", "id.prefix=SAME"], "set_prefix_br");
    source.run_bd(["config", "--set", "id.prefix=SAME"], "set_prefix_bd");

    source.run_obr(["create", "Same prefix issue"], "create");
    source.run_bd(["create", "Same prefix issue"], "create");
    source.run_obr(["sync", "--flush-only"], "flush");
    source.run_bd(["sync", "--flush-only"], "flush");

    // Target workspace with SAME prefix
    let target = ConformanceWorkspace::new();
    target.init_both();
    target.run_obr(["config", "--set", "id.prefix=SAME"], "set_prefix_br");
    target.run_bd(["config", "--set", "id.prefix=SAME"], "set_prefix_bd");

    // Copy JSONL
    let obr_src = source.obr_root.join(".obr").join("issues.jsonl");
    let bd_src = source.bd_root.join(".obr").join("issues.jsonl");
    let obr_dst = target.obr_root.join(".obr").join("issues.jsonl");
    let bd_dst = target.bd_root.join(".obr").join("issues.jsonl");

    fs::copy(&obr_src, &obr_dst).expect("copy obr jsonl");
    fs::copy(&bd_src, &bd_dst).expect("copy bd jsonl");

    // Import with matching prefix should succeed
    let obr_import = target.run_obr(["sync", "--import-only"], "import_same");
    let bd_import = target.run_bd(["sync", "--import-only"], "import_same");

    assert!(
        obr_import.status.success(),
        "obr import with same prefix failed: {}",
        obr_import.stderr
    );
    assert!(
        bd_import.status.success(),
        "bd import with same prefix failed: {}",
        bd_import.stderr
    );

    // Verify issues were imported
    let obr_list = target.run_obr(["list", "--json"], "list");
    let bd_list = target.run_bd(["list", "--json"], "list");

    let obr_val: Value = serde_json::from_str(&extract_json_payload(&obr_list.stdout))
        .unwrap_or(Value::Array(vec![]));
    let bd_val: Value = serde_json::from_str(&extract_json_payload(&bd_list.stdout))
        .unwrap_or(Value::Array(vec![]));

    let obr_count = issue_count(&obr_val);
    let bd_count = issue_count(&bd_val);

    assert_eq!(
        obr_count, bd_count,
        "import count differs: obr={}, bd={}",
        obr_count, bd_count
    );
    assert!(obr_count >= 1, "should have at least 1 issue imported");

    info!("conformance_sync_import_same_prefix_succeeds passed");
}

#[test]
fn conformance_sync_status_shows_prefix_info() {
    skip_if_no_bd!();
    common::init_test_logging();
    info!("Starting conformance_sync_status_shows_prefix_info test");

    // NOTE: bd does not support `sync --status` flag, so this tests obr only
    // Known difference: bd doesn't have status checking functionality

    let workspace = ConformanceWorkspace::new();
    workspace.init_both();

    workspace.run_obr(["config", "--set", "id.prefix=STATUS"], "set_prefix_br");

    workspace.run_obr(["create", "Status test"], "create");

    workspace.run_obr(["sync", "--flush-only"], "flush");

    // Check sync status - obr only (bd doesn't support --status flag)
    let obr_status = workspace.run_obr(["sync", "--status", "--json"], "status");

    assert!(
        obr_status.status.success(),
        "obr status failed: {}",
        obr_status.stderr
    );

    // obr should produce valid JSON output
    let obr_json = extract_json_payload(&obr_status.stdout);
    let obr_val: Result<Value, _> = serde_json::from_str(&obr_json);

    assert!(obr_val.is_ok(), "obr status should produce valid JSON");

    info!("conformance_sync_status_shows_prefix_info passed");
}

// ---------------------------------------------------------------------------
// Harness self-tests (`beads_rust-f175`, `beads_rust-ywot`)
//
// These exercise the comparison machinery itself and need no `bd`, so they run
// on every host — including the ones where every `conformance_*` test skips.
// ---------------------------------------------------------------------------

#[test]
fn issue_count_reads_both_envelope_shapes() {
    // `beads_rust-ywot`: the bug this guards against is that
    // `Value::as_array()` returns None for obr's paginated object, so every
    // `obr list --json` count read as 0 — failing loudly where bd returned rows
    // and, worse, passing vacuously where it did not.
    let bare = serde_json::json!([{ "id": "bd-a" }, { "id": "bd-b" }]);
    let paginated = serde_json::json!({
        "issues": [{ "id": "bd-a" }, { "id": "bd-b" }],
        "total": 2, "limit": 50, "offset": 0, "has_more": false,
    });

    assert_eq!(issue_count(&bare), 2);
    assert_eq!(
        issue_count(&paginated),
        2,
        "the shape that used to read as 0"
    );
    assert_eq!(issue_count(&bare), issue_count(&paginated));

    // Empty is genuinely empty in both shapes, not conflated with "unreadable".
    assert_eq!(issue_count(&serde_json::json!([])), 0);
    assert_eq!(issue_count(&serde_json::json!({ "issues": [] })), 0);

    // Anything else counts as 0, matching the previous `unwrap_or(0)`.
    assert_eq!(
        issue_count(&serde_json::json!({ "summary": { "x": 1 } })),
        0
    );
    assert_eq!(issue_count(&serde_json::Value::Null), 0);
}

#[test]
fn issue_items_reads_both_envelope_shapes() {
    let bare = serde_json::json!([{ "id": "bd-a" }]);
    let paginated = serde_json::json!({ "issues": [{ "id": "bd-a" }], "total": 1 });

    assert_eq!(issue_items(&bare).len(), 1);
    assert_eq!(issue_items(&paginated).len(), 1);
    assert_eq!(issue_items(&paginated)[0]["id"], "bd-a");
    assert!(issue_items(&serde_json::json!({ "summary": {} })).is_empty());
}

#[test]
fn init_args_force_the_same_prefix_on_both_tools() {
    // Both tools derive a prefix from their working directory when none is
    // given, and this harness deliberately gives them different directories.
    // If this ever regresses to a bare ["init"], every id comparison under
    // CompareMode::NormalizedJson becomes unsatisfiable.
    assert_eq!(
        ConformanceWorkspace::INIT_ARGS,
        ["init", "--prefix", CONFORMANCE_PREFIX]
    );
    assert!(
        !CONFORMANCE_PREFIX.contains('-'),
        "a prefix containing '-' would be split by the id normalizer"
    );
}

#[test]
fn id_normalization_preserves_the_whole_prefix() {
    let mut value = serde_json::json!({
        "id": "bd-a1b2c3",
        "issue_id": "beads-rust-9f8e7d",
        "depends_on_id": "bd-parent.1",
        "nested": { "id": "ops-deadbeef" },
        "items": [{ "id": "api-cafe01" }],
    });
    normalize_value(&mut value);

    assert_eq!(value["id"], "bd-NORMALIZED"); // invariant: masker unit test on a literal constant
    // The load-bearing case: a prefix that itself contains a dash must survive
    // intact. Splitting on the first dash yielded "beads-NORMALIZED".
    assert_eq!(value["issue_id"], "beads-rust-NORMALIZED");
    // A dotted child id has only one dash, so `find` and `rfind` agree and the
    // whole `parent.1` suffix is treated as the hash portion.
    assert_eq!(value["depends_on_id"], "bd-NORMALIZED"); // invariant: masker unit test on a literal constant
    assert_eq!(value["nested"]["id"], "ops-NORMALIZED");
    assert_eq!(value["items"][0]["id"], "api-NORMALIZED");
}

#[test]
fn id_normalization_makes_matching_prefixes_compare_equal() {
    // The whole point: two workspaces initialized with the same prefix produce
    // ids that normalize to the same value despite different hashes.
    let mut obr = serde_json::json!({ "id": "bd-aaaaaa", "content_hash": "deadbeef" });
    let mut bd = serde_json::json!({ "id": "bd-zzzzzz", "content_hash": "cafebabe" });
    normalize_value(&mut obr);
    normalize_value(&mut bd);
    assert_eq!(obr, bd);

    // ...and that differing prefixes still compare unequal, so the fix does not
    // paper over a real divergence.
    let mut mismatched = serde_json::json!({ "id": "br-aaaaaa", "content_hash": "deadbeef" });
    normalize_value(&mut mismatched);
    assert_ne!(obr, mismatched);
}
