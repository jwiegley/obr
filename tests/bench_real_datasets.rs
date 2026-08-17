//! Benchmark suite using real datasets from known repositories.
//!
//! This test module runs read-heavy and write-heavy workloads on real `.obr` datasets
//! and compares obr (Rust) vs bd (Go) performance. All operations run on isolated copies
//! to ensure source datasets are never mutated.
//!
//! # Usage
//!
//! Run all benchmarks:
//! ```bash
//! cargo test --test bench_real_datasets -- --nocapture --ignored
//! ```
//!
//! Run with artifact logging:
//! ```bash
//! HARNESS_ARTIFACTS=1 cargo test --test bench_real_datasets -- --nocapture --ignored
//! ```
//!
//! # Workloads
//!
//! Read-heavy:
//! - `list --json` - List all issues
//! - `ready --json` - Get ready issues (dependency resolution)
//! - `stats --json` - Project statistics
//! - `search "keyword" --json` - Full-text search
//!
//! Write-heavy:
//! - `create` - Create new issues
//! - `update` - Update issue fields
//! - `close` - Close issues with reason

#![allow(clippy::cast_precision_loss, clippy::similar_names)]

mod common;

use common::binary_discovery::{DiscoveredBinaries, discover_binaries};
use common::dataset_registry::{
    DatasetIntegrityGuard, DatasetMetadata, IsolatedDataset, KnownDataset,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

// =============================================================================
// Metrics Collection
// =============================================================================

/// Metrics captured for a single command run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMetrics {
    /// Command label (e.g., "list", "ready")
    pub label: String,
    /// Binary used ("obr" or "bd")
    pub binary: String,
    /// Wall-clock duration in milliseconds
    pub duration_ms: u128,
    /// Peak RSS in bytes (if available)
    pub peak_rss_bytes: Option<u64>,
    /// Exit code
    pub exit_code: i32,
    /// Whether command succeeded
    pub success: bool,
    /// Stdout length (proxy for output size)
    pub stdout_len: usize,
    /// Stderr length
    pub stderr_len: usize,
}

/// Comparison of obr vs bd for a single operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    pub label: String,
    pub obr: RunMetrics,
    pub bd: RunMetrics,
    /// obr/bd duration ratio (< 1.0 means obr is faster)
    pub duration_ratio: f64,
    /// obr/bd RSS ratio (< 1.0 means obr uses less memory)
    pub rss_ratio: Option<f64>,
}

/// Full benchmark results for a dataset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetBenchmark {
    pub dataset: DatasetMetadataSummary,
    pub comparisons: Vec<Comparison>,
    pub summary: BenchmarkSummary,
}

/// Lightweight metadata summary for output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMetadataSummary {
    pub name: String,
    pub issue_count: usize,
    pub dependency_count: usize,
    pub jsonl_size_bytes: u64,
    pub db_size_bytes: u64,
}

impl From<&DatasetMetadata> for DatasetMetadataSummary {
    fn from(m: &DatasetMetadata) -> Self {
        Self {
            name: m.name.clone(),
            issue_count: m.issue_count,
            dependency_count: m.dependency_count,
            jsonl_size_bytes: m.jsonl_size_bytes,
            db_size_bytes: m.db_size_bytes,
        }
    }
}

/// Summary statistics for a benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkSummary {
    /// Geometric mean of duration ratios (obr/bd)
    pub geomean_duration_ratio: f64,
    /// Geometric mean of RSS ratios (obr/bd)
    pub geomean_rss_ratio: Option<f64>,
    /// Number of operations where obr was faster
    pub obr_faster_count: usize,
    /// Number of operations where bd was faster
    pub bd_faster_count: usize,
    /// Total obr time (ms)
    pub total_obr_ms: u128,
    /// Total bd time (ms)
    pub total_bd_ms: u128,
}

#[derive(Debug, Clone)]
struct CapturedRun {
    metrics: RunMetrics,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
struct CreatedWriteWorkloads {
    comparisons: Vec<Comparison>,
    obr_created_ids: Vec<String>,
    bd_created_ids: Vec<String>,
}

// =============================================================================
// Command Runner with Metrics
// =============================================================================

/// Run a command and capture metrics including RSS.
fn run_with_capture(
    binary_path: &Path,
    args: &[&str],
    cwd: &Path,
    label: &str,
    binary_name: &str,
) -> CapturedRun {
    let start = Instant::now();

    let output = Command::new(binary_path)
        .args(args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run command");

    let duration = start.elapsed();

    // Try to get peak RSS from /proc/self/status on Linux
    // For more accurate measurements, we'd need to use getrusage or external tools
    let peak_rss_bytes = get_peak_rss_bytes();

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    CapturedRun {
        metrics: RunMetrics {
            label: label.to_string(),
            binary: binary_name.to_string(),
            duration_ms: duration.as_millis(),
            peak_rss_bytes,
            exit_code: output.status.code().unwrap_or(-1),
            success: output.status.success(),
            stdout_len: output.stdout.len(),
            stderr_len: output.stderr.len(),
        },
        stdout,
        stderr,
    }
}

fn run_with_metrics(
    binary_path: &Path,
    args: &[&str],
    cwd: &Path,
    label: &str,
    binary_name: &str,
) -> RunMetrics {
    let start = Instant::now();

    let output = Command::new(binary_path)
        .args(args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("Failed to run command");

    let duration = start.elapsed();

    RunMetrics {
        label: label.to_string(),
        binary: binary_name.to_string(),
        duration_ms: duration.as_millis(),
        peak_rss_bytes: get_peak_rss_bytes(),
        exit_code: output.status.code().unwrap_or(-1),
        success: output.status.success(),
        stdout_len: output.stdout.len(),
        stderr_len: output.stderr.len(),
    }
}

fn summarize_command_failure(run: &CapturedRun) -> String {
    let stderr = run.stderr.trim();
    if !stderr.is_empty() {
        return stderr.chars().take(300).collect();
    }

    let stdout = run.stdout.trim();
    if !stdout.is_empty() {
        return stdout.chars().take(300).collect();
    }

    "command produced no output".to_string()
}

fn ensure_command_succeeded(run: &CapturedRun, context: &str) -> Result<(), String> {
    if run.metrics.success {
        return Ok(());
    }

    Err(format!(
        "{context} failed with exit code {}: {}",
        run.metrics.exit_code,
        summarize_command_failure(run)
    ))
}

fn extract_issue_id(output: &str) -> Result<String, String> {
    let parsed: Value = serde_json::from_str(output)
        .map_err(|error| format!("failed to parse JSON output: {error}"))?;

    let id = match parsed {
        Value::Object(map) => map.get("id").and_then(Value::as_str).map(str::to_string),
        Value::Array(items) => items.first().and_then(|item| {
            item.as_object()
                .and_then(|map| map.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        }),
        _ => None,
    };

    id.ok_or_else(|| "JSON output did not contain an issue id".to_string())
}

fn push_aggregate_total(comparisons: &mut Vec<Comparison>, prefix: &str, total_label: &str) {
    let obr_total: u128 = comparisons
        .iter()
        .filter(|comparison| comparison.label.starts_with(prefix))
        .map(|comparison| comparison.obr.duration_ms)
        .sum();
    let bd_total: u128 = comparisons
        .iter()
        .filter(|comparison| comparison.label.starts_with(prefix))
        .map(|comparison| comparison.bd.duration_ms)
        .sum();

    comparisons.push(Comparison {
        label: total_label.to_string(),
        obr: RunMetrics {
            label: total_label.to_string(),
            binary: "obr".to_string(),
            duration_ms: obr_total,
            peak_rss_bytes: None,
            exit_code: 0,
            success: true,
            stdout_len: 0,
            stderr_len: 0,
        },
        bd: RunMetrics {
            label: total_label.to_string(),
            binary: "bd".to_string(),
            duration_ms: bd_total,
            peak_rss_bytes: None,
            exit_code: 0,
            success: true,
            stdout_len: 0,
            stderr_len: 0,
        },
        duration_ratio: if bd_total > 0 {
            obr_total as f64 / bd_total as f64
        } else {
            1.0
        },
        rss_ratio: None,
    });
}

/// Try to get peak RSS from the OS.
/// On Linux, reads /proc/self/status. Returns None on other platforms.
fn get_peak_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if line.starts_with("VmHWM:") {
                    // Format: "VmHWM:    123456 kB"
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2
                        && let Ok(kb) = parts[1].parse::<u64>()
                    {
                        return Some(kb * 1024);
                    }
                }
            }
        }
        None
    }

    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

// =============================================================================
// Benchmark Workloads
// =============================================================================

/// Run read-heavy workloads and return comparisons.
fn run_read_workloads(obr_path: &Path, bd_path: &Path, workspace: &Path) -> Vec<Comparison> {
    let mut comparisons = Vec::new();

    // List all issues
    let obr = run_with_metrics(obr_path, &["list", "--json"], workspace, "list", "obr");
    let bd = run_with_metrics(bd_path, &["list", "--json"], workspace, "list", "bd");
    comparisons.push(make_comparison("list", obr, bd));

    // List with status filter
    let obr = run_with_metrics(
        obr_path,
        &["list", "--status=open", "--json"],
        workspace,
        "list_open",
        "obr",
    );
    let bd = run_with_metrics(
        bd_path,
        &["list", "--status=open", "--json"],
        workspace,
        "list_open",
        "bd",
    );
    comparisons.push(make_comparison("list_open", obr, bd));

    // Ready issues (dependency resolution)
    let obr = run_with_metrics(obr_path, &["ready", "--json"], workspace, "ready", "obr");
    let bd = run_with_metrics(bd_path, &["ready", "--json"], workspace, "ready", "bd");
    comparisons.push(make_comparison("ready", obr, bd));

    // Stats
    let obr = run_with_metrics(obr_path, &["stats", "--json"], workspace, "stats", "obr");
    let bd = run_with_metrics(bd_path, &["stats", "--json"], workspace, "stats", "bd");
    comparisons.push(make_comparison("stats", obr, bd));

    // Search (common term likely to exist)
    let obr = run_with_metrics(
        obr_path,
        &["search", "test", "--json"],
        workspace,
        "search",
        "obr",
    );
    let bd = run_with_metrics(
        bd_path,
        &["search", "test", "--json"],
        workspace,
        "search",
        "bd",
    );
    comparisons.push(make_comparison("search", obr, bd));

    // Show a specific issue (first one if exists)
    // We use `list --json` output to find an ID, but for simplicity just try a common prefix
    let obr = run_with_metrics(
        obr_path,
        &["list", "--limit=1", "--json"],
        workspace,
        "list_one",
        "obr",
    );
    let bd = run_with_metrics(
        bd_path,
        &["list", "--limit=1", "--json"],
        workspace,
        "list_one",
        "bd",
    );
    comparisons.push(make_comparison("list_one", obr, bd));

    comparisons
}

/// Run write-heavy workloads and return comparisons.
/// Note: These modify the workspace, so they should be run on an isolated copy.
fn benchmark_create_workloads(
    obr_path: &Path,
    bd_path: &Path,
    obr_workspace: &Path,
    bd_workspace: &Path,
) -> Result<CreatedWriteWorkloads, String> {
    let mut comparisons = Vec::new();
    let mut obr_created_ids = Vec::new();
    let mut bd_created_ids = Vec::new();

    for i in 0..10 {
        let title = format!("Benchmark issue {i}");
        let obr = run_with_capture(
            obr_path,
            &[
                "create",
                "--title",
                &title,
                "--type=task",
                "--priority=2",
                "--json",
            ],
            obr_workspace,
            &format!("create_{i}"),
            "obr",
        );
        let bd = run_with_capture(
            bd_path,
            &[
                "create",
                "--title",
                &title,
                "--type=task",
                "--priority=2",
                "--json",
            ],
            bd_workspace,
            &format!("create_{i}"),
            "bd",
        );

        ensure_command_succeeded(&obr, &format!("obr create_{i}"))?;
        ensure_command_succeeded(&bd, &format!("bd create_{i}"))?;

        let obr_id = extract_issue_id(&obr.stdout).map_err(|error| {
            format!(
                "failed to capture created obr issue id for create_{i}: {error}; stderr: {}",
                obr.stderr.trim()
            )
        })?;
        let bd_id = extract_issue_id(&bd.stdout).map_err(|error| {
            format!(
                "failed to capture created bd issue id for create_{i}: {error}; stderr: {}",
                bd.stderr.trim()
            )
        })?;

        obr_created_ids.push(obr_id);
        bd_created_ids.push(bd_id);
        comparisons.push(make_comparison(
            &format!("create_{i}"),
            obr.metrics,
            bd.metrics,
        ));
    }

    push_aggregate_total(&mut comparisons, "create_", "create_10_total");
    Ok(CreatedWriteWorkloads {
        comparisons,
        obr_created_ids,
        bd_created_ids,
    })
}

fn benchmark_update_workloads(
    obr_path: &Path,
    bd_path: &Path,
    obr_workspace: &Path,
    bd_workspace: &Path,
    obr_created_ids: &[String],
    bd_created_ids: &[String],
) -> Result<Vec<Comparison>, String> {
    let mut comparisons = Vec::new();

    for (i, (obr_id, bd_id)) in obr_created_ids.iter().zip(bd_created_ids).enumerate() {
        let update_title = format!("Benchmark issue {i} updated");
        let obr = run_with_capture(
            obr_path,
            &[
                "update",
                obr_id,
                "--title",
                &update_title,
                "--priority=1",
                "--json",
            ],
            obr_workspace,
            &format!("update_{i}"),
            "obr",
        );
        let bd = run_with_capture(
            bd_path,
            &[
                "update",
                bd_id,
                "--title",
                &update_title,
                "--priority=1",
                "--json",
            ],
            bd_workspace,
            &format!("update_{i}"),
            "bd",
        );

        ensure_command_succeeded(&obr, &format!("obr update_{i}"))?;
        ensure_command_succeeded(&bd, &format!("bd update_{i}"))?;
        comparisons.push(make_comparison(
            &format!("update_{i}"),
            obr.metrics,
            bd.metrics,
        ));
    }

    push_aggregate_total(&mut comparisons, "update_", "update_10_total");
    Ok(comparisons)
}

fn benchmark_close_workloads(
    obr_path: &Path,
    bd_path: &Path,
    obr_workspace: &Path,
    bd_workspace: &Path,
    obr_created_ids: &[String],
    bd_created_ids: &[String],
) -> Result<Vec<Comparison>, String> {
    let mut comparisons = Vec::new();

    for (i, (obr_id, bd_id)) in obr_created_ids.iter().zip(bd_created_ids).enumerate() {
        let obr = run_with_capture(
            obr_path,
            &["close", obr_id, "--reason", "benchmark close", "--json"],
            obr_workspace,
            &format!("close_{i}"),
            "obr",
        );
        let bd = run_with_capture(
            bd_path,
            &["close", bd_id, "--reason", "benchmark close", "--json"],
            bd_workspace,
            &format!("close_{i}"),
            "bd",
        );

        ensure_command_succeeded(&obr, &format!("obr close_{i}"))?;
        ensure_command_succeeded(&bd, &format!("bd close_{i}"))?;
        comparisons.push(make_comparison(
            &format!("close_{i}"),
            obr.metrics,
            bd.metrics,
        ));
    }

    push_aggregate_total(&mut comparisons, "close_", "close_10_total");
    Ok(comparisons)
}

fn run_write_workloads(
    obr_path: &Path,
    bd_path: &Path,
    obr_workspace: &Path,
    bd_workspace: &Path,
) -> Result<Vec<Comparison>, String> {
    let CreatedWriteWorkloads {
        mut comparisons,
        obr_created_ids,
        bd_created_ids,
    } = benchmark_create_workloads(obr_path, bd_path, obr_workspace, bd_workspace)?;
    comparisons.extend(benchmark_update_workloads(
        obr_path,
        bd_path,
        obr_workspace,
        bd_workspace,
        &obr_created_ids,
        &bd_created_ids,
    )?);
    comparisons.extend(benchmark_close_workloads(
        obr_path,
        bd_path,
        obr_workspace,
        bd_workspace,
        &obr_created_ids,
        &bd_created_ids,
    )?);
    Ok(comparisons)
}

/// Create a comparison from obr and bd metrics.
fn make_comparison(label: &str, obr: RunMetrics, bd: RunMetrics) -> Comparison {
    let duration_ratio = if bd.duration_ms > 0 {
        obr.duration_ms as f64 / bd.duration_ms as f64
    } else if obr.duration_ms > 0 {
        f64::INFINITY
    } else {
        1.0
    };

    let rss_ratio = match (obr.peak_rss_bytes, bd.peak_rss_bytes) {
        (Some(obr_rss), Some(bd_rss)) if bd_rss > 0 => Some(obr_rss as f64 / bd_rss as f64),
        _ => None,
    };

    Comparison {
        label: label.to_string(),
        obr,
        bd,
        duration_ratio,
        rss_ratio,
    }
}

/// Calculate summary statistics from comparisons.
fn calculate_summary(comparisons: &[Comparison]) -> BenchmarkSummary {
    let duration_ratios: Vec<f64> = comparisons
        .iter()
        .filter(|c| c.duration_ratio.is_finite() && c.duration_ratio > 0.0)
        .map(|c| c.duration_ratio)
        .collect();

    let rss_ratios: Vec<f64> = comparisons
        .iter()
        .filter_map(|c| c.rss_ratio)
        .filter(|r| r.is_finite() && *r > 0.0)
        .collect();

    #[allow(clippy::cast_precision_loss)]
    let geomean_duration_ratio = if duration_ratios.is_empty() {
        1.0
    } else {
        let log_sum: f64 = duration_ratios.iter().map(|r| r.ln()).sum();
        (log_sum / duration_ratios.len() as f64).exp()
    };

    #[allow(clippy::cast_precision_loss)]
    let geomean_rss_ratio = if rss_ratios.is_empty() {
        None
    } else {
        let log_sum: f64 = rss_ratios.iter().map(|r| r.ln()).sum();
        Some((log_sum / rss_ratios.len() as f64).exp())
    };

    let obr_faster_count = comparisons
        .iter()
        .filter(|c| c.duration_ratio < 1.0)
        .count();
    let bd_faster_count = comparisons
        .iter()
        .filter(|c| c.duration_ratio > 1.0)
        .count();

    let total_obr_ms: u128 = comparisons.iter().map(|c| c.obr.duration_ms).sum();
    let total_bd_ms: u128 = comparisons.iter().map(|c| c.bd.duration_ms).sum();

    BenchmarkSummary {
        geomean_duration_ratio,
        geomean_rss_ratio,
        obr_faster_count,
        bd_faster_count,
        total_obr_ms,
        total_bd_ms,
    }
}

// =============================================================================
// Output Formatting
// =============================================================================

/// Print a comparison table to stdout.
fn print_comparison_table(benchmark: &DatasetBenchmark) {
    let sep = "=".repeat(80);
    let dash = "-".repeat(80);
    let name = &benchmark.dataset.name;
    let issues = benchmark.dataset.issue_count;
    let deps = benchmark.dataset.dependency_count;
    let jsonl_kb = benchmark.dataset.jsonl_size_bytes as f64 / 1024.0;
    let db_kb = benchmark.dataset.db_size_bytes as f64 / 1024.0;

    println!("\n{sep}");
    println!(
        "Dataset: {name} ({issues} issues, {deps} deps, {jsonl_kb:.1} KB JSONL, {db_kb:.1} KB DB)"
    );
    println!("{sep}");
    println!(
        "{:<20} {:>12} {:>12} {:>12} {:>10}",
        "Operation", "obr (ms)", "bd (ms)", "Ratio", "Winner"
    );
    println!("{dash}");

    for c in &benchmark.comparisons {
        let winner = if c.duration_ratio < 0.95 {
            "obr"
        } else if c.duration_ratio > 1.05 {
            "bd"
        } else {
            "tie"
        };

        let label = &c.label;
        let obr_ms = c.obr.duration_ms;
        let bd_ms = c.bd.duration_ms;
        let ratio = c.duration_ratio;
        println!("{label:<20} {obr_ms:>12} {bd_ms:>12} {ratio:>12.2} {winner:>10}");
    }

    let faster = benchmark.summary.obr_faster_count;
    let total_ops = benchmark.comparisons.len();
    let geomean = benchmark.summary.geomean_duration_ratio;
    let obr_time = benchmark.summary.total_obr_ms;
    let bd_time = benchmark.summary.total_bd_ms;

    println!("{dash}");
    println!("Summary: obr faster in {faster}/{total_ops} ops, geomean ratio: {geomean:.2}x");
    println!("Total time: obr={obr_time}ms, bd={bd_time}ms");
    println!();
}

/// Write benchmark results to JSON file.
fn write_results_json(benchmarks: &[DatasetBenchmark], output_path: &Path) -> std::io::Result<()> {
    let file = File::create(output_path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, benchmarks)?;
    Ok(())
}

// =============================================================================
// Main Benchmark Runner
// =============================================================================

/// Run benchmarks on a single dataset.
fn benchmark_dataset(
    dataset: KnownDataset,
    binaries: &DiscoveredBinaries,
) -> Result<DatasetBenchmark, String> {
    let bd = binaries.require_bd()?;

    // Create integrity guard to ensure source isn't mutated
    let mut guard = DatasetIntegrityGuard::new(dataset)
        .map_err(|e| format!("Failed to create integrity guard: {e}"))?;

    let before = guard.verify_before();
    if !before.passed {
        return Err(format!("Source integrity check failed: {}", before.message));
    }

    // Create isolated copies for obr and bd
    let obr_isolated = IsolatedDataset::from_dataset(dataset)
        .map_err(|e| format!("Failed to create obr workspace: {e}"))?;
    let bd_isolated = IsolatedDataset::from_dataset(dataset)
        .map_err(|e| format!("Failed to create bd workspace: {e}"))?;

    let metadata_summary = DatasetMetadataSummary::from(&obr_isolated.metadata);

    let ds_name = dataset.name();
    let issue_count = obr_isolated.metadata.issue_count;
    println!("\nBenchmarking {ds_name} ({issue_count} issues)...");

    // Run read workloads (same workspace is fine since reads don't modify)
    let mut comparisons =
        run_read_workloads(&binaries.obr.path, &bd.path, obr_isolated.workspace_root());

    // Run write workloads (separate workspaces since writes modify state)
    let write_comparisons = run_write_workloads(
        &binaries.obr.path,
        &bd.path,
        obr_isolated.workspace_root(),
        bd_isolated.workspace_root(),
    )?;
    comparisons.extend(write_comparisons);

    // Verify source wasn't mutated
    let after = guard.verify_after();
    if !after.passed {
        return Err(format!(
            "Source dataset was mutated during benchmark: {}",
            after.message
        ));
    }

    let summary = calculate_summary(&comparisons);

    Ok(DatasetBenchmark {
        dataset: metadata_summary,
        comparisons,
        summary,
    })
}

// =============================================================================
// Tests
// =============================================================================

/// Run benchmarks on all available datasets.
#[test]
#[ignore = "run with: cargo test --test bench_real_datasets -- --ignored --nocapture"]
fn benchmark_all_datasets() {
    println!("\n=== Real Dataset Benchmark Suite ===\n");

    // Discover binaries
    let binaries = match discover_binaries() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Binary discovery failed: {e}");
            panic!("Cannot run benchmarks without obr binary");
        }
    };

    println!(
        "obr: {} ({})",
        binaries.obr.path.display(),
        binaries.obr.version
    );
    if let Some(ref bd) = binaries.bd {
        println!("bd: {} ({})", bd.path.display(), bd.version);
    } else {
        println!("bd: NOT FOUND - skipping obr/bd comparisons");
        println!("Install bd from: https://github.com/steveyegge/beads");
        return;
    }

    let mut results: Vec<DatasetBenchmark> = Vec::new();

    for dataset in KnownDataset::all() {
        // Check if dataset exists
        if !dataset.obr_dir().exists() {
            let name = dataset.name();
            println!("\nSkipping {name} (not available)");
            continue;
        }

        match benchmark_dataset(*dataset, &binaries) {
            Ok(benchmark) => {
                print_comparison_table(&benchmark);
                results.push(benchmark);
            }
            Err(e) => {
                let name = dataset.name();
                eprintln!("\nFailed to benchmark {name}: {e}");
            }
        }
    }

    // Write JSON results
    if !results.is_empty() {
        let output_dir = PathBuf::from("target/benchmark-results");
        fs::create_dir_all(&output_dir).expect("create output dir");

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let output_path = output_dir.join(format!("real_datasets_{timestamp}.json"));

        if let Err(e) = write_results_json(&results, &output_path) {
            eprintln!("Failed to write results: {e}");
        } else {
            println!("\nResults written to: {}", output_path.display());
        }

        // Also write latest.json for easy access
        let latest_path = output_dir.join("real_datasets_latest.json");
        if let Err(e) = write_results_json(&results, &latest_path) {
            eprintln!("Failed to write latest results: {e}");
        }
    }

    // Print overall summary
    if !results.is_empty() {
        println!("\n{}", "=".repeat(80));
        println!("OVERALL SUMMARY");
        println!("{}", "=".repeat(80));

        let all_ratios: Vec<f64> = results
            .iter()
            .flat_map(|b| b.comparisons.iter().map(|c| c.duration_ratio))
            .filter(|r| r.is_finite() && *r > 0.0)
            .collect();

        if !all_ratios.is_empty() {
            let log_sum: f64 = all_ratios.iter().map(|r| r.ln()).sum();
            #[allow(clippy::cast_precision_loss)]
            let overall_geomean = (log_sum / all_ratios.len() as f64).exp();

            let total_obr: u128 = results.iter().map(|b| b.summary.total_obr_ms).sum();
            let total_bd: u128 = results.iter().map(|b| b.summary.total_bd_ms).sum();

            let dataset_count = results.len();
            let op_count = all_ratios.len();
            println!("Datasets benchmarked: {dataset_count}");
            println!("Total operations: {op_count}");
            println!("Overall geomean ratio: {overall_geomean:.2}x");
            println!("Total obr time: {total_obr}ms");
            println!("Total bd time: {total_bd}ms");

            if overall_geomean < 1.0 {
                let pct = (1.0 - overall_geomean) * 100.0;
                println!("\nbr is {pct:.1}% faster than bd on average");
            } else {
                let pct = (overall_geomean - 1.0) * 100.0;
                println!("\nbd is {pct:.1}% faster than obr on average");
            }
        }
    }
}

/// Benchmark only `obr` dataset (faster iteration).
#[test]
#[ignore = "manual benchmark run"]
fn benchmark_obr_only() {
    let binaries = discover_binaries().expect("Binary discovery failed");

    if binaries.bd.is_none() {
        println!("bd not found, skipping benchmark");
        return;
    }

    let benchmark = benchmark_dataset(KnownDataset::Obr, &binaries).expect("Benchmark failed");

    print_comparison_table(&benchmark);
}

/// Unit test for metrics calculation.
#[test]
fn test_calculate_summary() {
    let comparisons = vec![
        Comparison {
            label: "op1".to_string(),
            obr: RunMetrics {
                label: "op1".to_string(),
                binary: "obr".to_string(),
                duration_ms: 100,
                peak_rss_bytes: Some(1000),
                exit_code: 0,
                success: true,
                stdout_len: 10,
                stderr_len: 0,
            },
            bd: RunMetrics {
                label: "op1".to_string(),
                binary: "bd".to_string(),
                duration_ms: 200,
                peak_rss_bytes: Some(2000),
                exit_code: 0,
                success: true,
                stdout_len: 10,
                stderr_len: 0,
            },
            duration_ratio: 0.5,
            rss_ratio: Some(0.5),
        },
        Comparison {
            label: "op2".to_string(),
            obr: RunMetrics {
                label: "op2".to_string(),
                binary: "obr".to_string(),
                duration_ms: 200,
                peak_rss_bytes: Some(2000),
                exit_code: 0,
                success: true,
                stdout_len: 10,
                stderr_len: 0,
            },
            bd: RunMetrics {
                label: "op2".to_string(),
                binary: "bd".to_string(),
                duration_ms: 100,
                peak_rss_bytes: Some(1000),
                exit_code: 0,
                success: true,
                stdout_len: 10,
                stderr_len: 0,
            },
            duration_ratio: 2.0,
            rss_ratio: Some(2.0),
        },
    ];

    let summary = calculate_summary(&comparisons);

    // Geomean of 0.5 and 2.0 is 1.0
    assert!((summary.geomean_duration_ratio - 1.0).abs() < 0.01);
    assert_eq!(summary.obr_faster_count, 1);
    assert_eq!(summary.bd_faster_count, 1);
    assert_eq!(summary.total_obr_ms, 300);
    assert_eq!(summary.total_bd_ms, 300);
}

/// Unit test for comparison creation.
#[test]
fn test_make_comparison() {
    let obr = RunMetrics {
        label: "test".to_string(),
        binary: "obr".to_string(),
        duration_ms: 100,
        peak_rss_bytes: Some(1024),
        exit_code: 0,
        success: true,
        stdout_len: 50,
        stderr_len: 0,
    };

    let bd = RunMetrics {
        label: "test".to_string(),
        binary: "bd".to_string(),
        duration_ms: 200,
        peak_rss_bytes: Some(2048),
        exit_code: 0,
        success: true,
        stdout_len: 50,
        stderr_len: 0,
    };

    let comparison = make_comparison("test", obr, bd);

    assert_eq!(comparison.label, "test");
    assert!((comparison.duration_ratio - 0.5).abs() < 0.01);
    assert!((comparison.rss_ratio.unwrap() - 0.5).abs() < 0.01);
}

#[test]
fn test_extract_issue_id_from_object_output() {
    let output = r#"{"id":"bd-123","title":"example"}"#;
    assert_eq!(extract_issue_id(output).unwrap(), "bd-123");
}

#[test]
fn test_extract_issue_id_from_array_output() {
    let output = r#"[{"id":"bd-456","title":"example"}]"#;
    assert_eq!(extract_issue_id(output).unwrap(), "bd-456");
}
