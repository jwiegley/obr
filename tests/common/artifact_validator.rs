//! Artifact Log Validator
//!
//! Validates JSONL event logs, snapshot files, and summaries against the
//! shapes this module defines; the schema is asserted here in code rather than
//! mirrored in prose.
//!
//! Task: beads_rust-r23m

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const STARTUP_MATRIX_MANIFEST: &str = "startup-matrix-manifest.json";
const PERF_EVIDENCE_MANIFEST: &str = "perf-evidence-manifest.json";
const REQUIRED_STARTUP_STATES: &[&str] = &[
    "clean",
    "stale",
    "routed",
    "no_db",
    "read_only_fast_open",
    "sync_status",
    "recovery_anomaly",
];

/// Validation result with detailed error context
#[derive(Debug)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<String>,
}

impl ValidationResult {
    pub const fn ok() -> Self {
        Self {
            valid: true,
            errors: vec![],
            warnings: vec![],
        }
    }

    pub fn with_error(mut self, error: ValidationError) -> Self {
        self.valid = false;
        self.errors.push(error);
        self
    }

    pub fn with_warning(mut self, warning: String) -> Self {
        self.warnings.push(warning);
        self
    }

    pub fn merge(mut self, other: Self) -> Self {
        self.valid = self.valid && other.valid;
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
        self
    }
}

/// Detailed validation error
#[derive(Debug)]
pub struct ValidationError {
    pub line: Option<usize>,
    pub field: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.line, &self.field) {
            (Some(line), Some(field)) => {
                write!(f, "Line {}, field '{}': {}", line, field, self.message)
            }
            (Some(line), None) => write!(f, "Line {}: {}", line, self.message),
            (None, Some(field)) => write!(f, "Field '{}': {}", field, self.message),
            (None, None) => write!(f, "{}", self.message),
        }
    }
}

/// Event types in the log
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Command,
    Snapshot,
}

/// JSONL event entry - matches `harness::RunEvent`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub timestamp: String,
    pub event_type: String,
    pub label: String,
    pub binary: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub exit_code: i32,
    pub success: bool,
    pub duration_ms: u128,
    pub stdout_len: usize,
    pub stderr_len: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<String>,
}

/// File entry in snapshot files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
}

/// Test summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub suite: String,
    pub test: String,
    pub passed: bool,
    pub run_count: usize,
    pub timestamp: String,
}

/// Manifest for storage-open/startup matrix performance bundles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupMatrixManifest {
    pub schema_version: String,
    pub matrix_name: String,
    pub generated_at: String,
    pub states: Vec<StartupMatrixState>,
    pub aggregation: StartupMatrixAggregation,
}

/// One startup state measured by the matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupMatrixState {
    pub state: String,
    pub command_log_path: String,
    pub timing_summary_path: String,
    pub syscall_summary_path: String,
    pub rss_summary_path: String,
    #[serde(default)]
    pub raw_artifact_paths: Vec<String>,
}

/// Aggregation outcome for a startup matrix bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupMatrixAggregation {
    pub status: String,
    pub raw_evidence_preserved: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Manifest for reusable performance evidence and tail-latency gate bundles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceManifest {
    pub schema_version: String,
    pub generated_at: String,
    #[serde(default)]
    pub valid_until: Option<String>,
    pub command: PerfEvidenceCommand,
    pub dataset: PerfEvidenceDataset,
    pub git: PerfEvidenceGit,
    pub binary: PerfEvidenceBinary,
    pub environment: PerfEvidenceEnvironment,
    pub timing: PerfEvidenceTiming,
    pub resources: PerfEvidenceResources,
    pub golden: PerfEvidenceGolden,
    pub isomorphism_note_path: String,
    pub policy: PerfEvidencePolicy,
    pub comparison: PerfEvidenceComparison,
    #[serde(default)]
    pub raw_artifact_paths: Vec<String>,
}

/// Command covered by a performance evidence bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceCommand {
    pub label: String,
    pub args: Vec<String>,
}

/// Dataset fingerprint used by the measured command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceDataset {
    pub name: String,
    #[serde(default)]
    pub issue_count: Option<usize>,
    #[serde(default)]
    pub content_hash: Option<String>,
}

/// Git revision metadata for reproducibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceGit {
    pub revision: String,
    pub dirty: bool,
}

/// Binary metadata for the measured executable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceBinary {
    pub path: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// Environment fingerprint for a performance evidence run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceEnvironment {
    pub os: String,
    #[serde(default)]
    pub rustc: Option<String>,
    #[serde(default)]
    pub env: Vec<PerfEvidenceEnvVar>,
}

/// Environment variable recorded without leaking raw values by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceEnvVar {
    pub name: String,
    #[serde(default)]
    pub value_hash: Option<String>,
}

/// Timing summary and raw sample references for tail-latency gates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceTiming {
    pub sample_count: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub summary_path: String,
    pub raw_samples_path: String,
}

/// Resource evidence references.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceResources {
    #[serde(rename = "syscall_summary_path")]
    pub syscalls: String,
    #[serde(rename = "io_summary_path")]
    pub io: String,
    #[serde(rename = "rss_summary_path")]
    pub rss: String,
}

/// Golden output references and hashes for behavior-preservation checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceGolden {
    pub stdout_sha256: String,
    #[serde(default)]
    pub stderr_sha256: Option<String>,
    pub checksums_path: String,
    pub stdout_path: String,
    #[serde(default)]
    pub stderr_path: Option<String>,
}

/// Advisory or enforcing gate policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidencePolicy {
    pub mode: String,
    #[serde(default)]
    pub baseline_manifest_path: Option<String>,
    #[serde(default)]
    pub latency_regression_budget_pct: Option<f64>,
    #[serde(default)]
    pub syscall_regression_budget_pct: Option<f64>,
    pub output_hash_must_match: bool,
}

/// Baseline-vs-candidate comparison result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfEvidenceComparison {
    pub status: String,
    #[serde(default)]
    pub baseline_manifest_path: Option<String>,
    #[serde(default)]
    pub p95_delta_pct: Option<f64>,
    #[serde(default)]
    pub stdout_hash_match: Option<bool>,
    #[serde(default)]
    pub syscall_delta_pct: Option<f64>,
    pub decision_reason: String,
}

/// Artifact validator
pub struct ArtifactValidator {
    strict: bool,
}

impl Default for ArtifactValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtifactValidator {
    pub const fn new() -> Self {
        Self { strict: true }
    }

    /// Enable/disable strict mode (fails on warnings)
    pub const fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Validate an events.jsonl file
    pub fn validate_events_file(&self, path: &Path) -> ValidationResult {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return ValidationResult::ok().with_error(ValidationError {
                    line: None,
                    field: None,
                    message: format!("Failed to read file: {e}"),
                });
            }
        };

        self.validate_events_content(&content)
    }

    /// Validate events content (JSONL)
    pub fn validate_events_content(&self, content: &str) -> ValidationResult {
        let mut result = ValidationResult::ok();

        // Normalize line endings
        let content = content.replace("\r\n", "\n");

        for (idx, line) in content.lines().enumerate() {
            let line_num = idx + 1;
            let line = line.trim();

            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<RunEvent>(line) {
                Ok(event) => {
                    result = result.merge(self.validate_event(&event, line_num));
                }
                Err(e) => {
                    result = result.with_error(ValidationError {
                        line: Some(line_num),
                        field: None,
                        message: format!("Invalid JSON: {e}"),
                    });
                }
            }
        }

        result
    }

    /// Validate a single event
    #[allow(clippy::unused_self)]
    fn validate_event(&self, event: &RunEvent, line_num: usize) -> ValidationResult {
        let mut result = ValidationResult::ok();

        // Validate timestamp (RFC3339)
        if DateTime::parse_from_rfc3339(&event.timestamp).is_err() {
            result = result.with_error(ValidationError {
                line: Some(line_num),
                field: Some("timestamp".to_string()),
                message: format!("Invalid RFC3339 timestamp: {}", event.timestamp),
            });
        }

        // Validate event_type
        if event.event_type != "command" && event.event_type != "snapshot" {
            result = result.with_error(ValidationError {
                line: Some(line_num),
                field: Some("event_type".to_string()),
                message: format!("Must be 'command' or 'snapshot', got: {}", event.event_type),
            });
        }

        // Validate label is non-empty
        if event.label.is_empty() {
            result = result.with_error(ValidationError {
                line: Some(line_num),
                field: Some("label".to_string()),
                message: "Label cannot be empty".to_string(),
            });
        }

        // Validate cwd is absolute
        if !event.cwd.is_empty() && !event.cwd.starts_with('/') && !event.cwd.contains(':') {
            result = result.with_warning(format!(
                "Line {}: cwd should be absolute path: {}",
                line_num, event.cwd
            ));
        }

        // For command events, validate binary is set
        if event.event_type == "command" && event.binary.is_empty() {
            result = result.with_error(ValidationError {
                line: Some(line_num),
                field: Some("binary".to_string()),
                message: "Binary required for command events".to_string(),
            });
        }

        // Validate exit code range
        if event.exit_code < -128 || event.exit_code > 255 {
            result = result.with_warning(format!(
                "Line {}: exit_code {} outside typical range [-128, 255]",
                line_num, event.exit_code
            ));
        }

        // Validate path safety (no traversal)
        for path in [&event.stdout_path, &event.stderr_path, &event.snapshot_path]
            .into_iter()
            .flatten()
        {
            if path.contains("..") {
                result = result.with_error(ValidationError {
                    line: Some(line_num),
                    field: Some("*_path".to_string()),
                    message: format!("Path traversal detected: {path}"),
                });
            }
        }

        result
    }

    /// Validate a snapshot file
    pub fn validate_snapshot_file(&self, path: &Path) -> ValidationResult {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return ValidationResult::ok().with_error(ValidationError {
                    line: None,
                    field: None,
                    message: format!("Failed to read file: {e}"),
                });
            }
        };

        self.validate_snapshot_content(&content)
    }

    /// Validate snapshot content
    #[allow(clippy::unused_self)]
    pub fn validate_snapshot_content(&self, content: &str) -> ValidationResult {
        let mut result = ValidationResult::ok();

        let entries: Vec<FileEntry> = match serde_json::from_str(content) {
            Ok(e) => e,
            Err(e) => {
                return result.with_error(ValidationError {
                    line: None,
                    field: None,
                    message: format!("Invalid JSON array: {e}"),
                });
            }
        };

        for (idx, entry) in entries.iter().enumerate() {
            // Validate path is relative
            if entry.path.starts_with('/') || entry.path.contains(':') {
                result = result.with_error(ValidationError {
                    line: Some(idx + 1),
                    field: Some("path".to_string()),
                    message: format!("Path must be relative: {}", entry.path),
                });
            }

            // Validate no traversal
            if entry.path.contains("..") {
                result = result.with_error(ValidationError {
                    line: Some(idx + 1),
                    field: Some("path".to_string()),
                    message: format!("Path traversal detected: {}", entry.path),
                });
            }

            // Warn if directory has non-zero size
            if entry.is_dir && entry.size > 0 {
                result = result.with_warning(format!(
                    "Entry {}: directory '{}' has non-zero size {}",
                    idx + 1,
                    entry.path,
                    entry.size
                ));
            }
        }

        // Check for sorted order
        let mut sorted = entries.clone();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        if entries.iter().map(|e| &e.path).collect::<Vec<_>>()
            != sorted.iter().map(|e| &e.path).collect::<Vec<_>>()
        {
            result = result.with_warning("Entries not sorted by path".to_string());
        }

        result
    }

    /// Validate a summary file
    pub fn validate_summary_file(&self, path: &Path) -> ValidationResult {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return ValidationResult::ok().with_error(ValidationError {
                    line: None,
                    field: None,
                    message: format!("Failed to read file: {e}"),
                });
            }
        };

        self.validate_summary_content(&content)
    }

    /// Validate summary content
    #[allow(clippy::unused_self)]
    pub fn validate_summary_content(&self, content: &str) -> ValidationResult {
        let mut result = ValidationResult::ok();

        let summary: Summary = match serde_json::from_str(content) {
            Ok(s) => s,
            Err(e) => {
                return result.with_error(ValidationError {
                    line: None,
                    field: None,
                    message: format!("Invalid JSON: {e}"),
                });
            }
        };

        // Validate timestamp
        if DateTime::parse_from_rfc3339(&summary.timestamp).is_err() {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("timestamp".to_string()),
                message: format!("Invalid RFC3339 timestamp: {}", summary.timestamp),
            });
        }

        // Validate suite name
        if summary.suite.is_empty() {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("suite".to_string()),
                message: "Suite name cannot be empty".to_string(),
            });
        }

        // Validate test name
        if summary.test.is_empty() {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("test".to_string()),
                message: "Test name cannot be empty".to_string(),
            });
        }

        result
    }

    /// Validate a startup matrix manifest file.
    pub fn validate_startup_matrix_manifest_file(&self, path: &Path) -> ValidationResult {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return ValidationResult::ok().with_error(ValidationError {
                    line: None,
                    field: None,
                    message: format!("Failed to read file: {e}"),
                });
            }
        };

        self.validate_startup_matrix_manifest_content(&content)
    }

    /// Validate startup matrix manifest JSON content.
    pub fn validate_startup_matrix_manifest_content(&self, content: &str) -> ValidationResult {
        let manifest = match Self::parse_startup_matrix_manifest(content) {
            Ok(manifest) => manifest,
            Err(error) => return ValidationResult::ok().with_error(error),
        };

        self.validate_startup_matrix_manifest(&manifest)
    }

    fn parse_startup_matrix_manifest(
        content: &str,
    ) -> Result<StartupMatrixManifest, ValidationError> {
        serde_json::from_str(content).map_err(|e| ValidationError {
            line: None,
            field: None,
            message: format!("Invalid startup matrix manifest JSON: {e}"),
        })
    }

    fn validate_startup_matrix_manifest(
        &self,
        manifest: &StartupMatrixManifest,
    ) -> ValidationResult {
        let mut result = ValidationResult::ok();

        if manifest.schema_version.trim().is_empty() {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("schema_version".to_string()),
                message: "Schema version cannot be empty".to_string(),
            });
        }

        if manifest.matrix_name.trim().is_empty() {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("matrix_name".to_string()),
                message: "Matrix name cannot be empty".to_string(),
            });
        }

        if DateTime::parse_from_rfc3339(&manifest.generated_at).is_err() {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("generated_at".to_string()),
                message: format!("Invalid RFC3339 timestamp: {}", manifest.generated_at),
            });
        }

        result = result.merge(Self::validate_startup_matrix_states(&manifest.states));
        result = result.merge(self.validate_startup_matrix_aggregation(manifest));

        result
    }

    fn validate_startup_matrix_states(states: &[StartupMatrixState]) -> ValidationResult {
        let mut result = ValidationResult::ok();
        let mut seen = BTreeSet::new();

        for state in states {
            if !seen.insert(state.state.as_str()) {
                result = result.with_error(ValidationError {
                    line: None,
                    field: Some("states".to_string()),
                    message: format!("Duplicate startup state: {}", state.state),
                });
            }

            for (field, path) in [
                ("command_log_path", &state.command_log_path),
                ("timing_summary_path", &state.timing_summary_path),
                ("syscall_summary_path", &state.syscall_summary_path),
                ("rss_summary_path", &state.rss_summary_path),
            ] {
                result = result.merge(Self::validate_relative_artifact_path(field, path));
            }

            for path in &state.raw_artifact_paths {
                result = result.merge(Self::validate_relative_artifact_path(
                    "raw_artifact_paths",
                    path,
                ));
            }
        }

        for required in REQUIRED_STARTUP_STATES {
            if !seen.contains(required) {
                result = result.with_error(ValidationError {
                    line: None,
                    field: Some("states".to_string()),
                    message: format!("Missing required startup state: {required}"),
                });
            }
        }

        result
    }

    #[allow(clippy::unused_self)]
    fn validate_startup_matrix_aggregation(
        &self,
        manifest: &StartupMatrixManifest,
    ) -> ValidationResult {
        let mut result = ValidationResult::ok();
        let status = manifest.aggregation.status.as_str();

        if !matches!(status, "ok" | "partial" | "failed") {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("aggregation.status".to_string()),
                message: format!("Must be 'ok', 'partial', or 'failed', got: {status}"),
            });
        }

        if status != "ok" && !manifest.aggregation.raw_evidence_preserved {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("aggregation.raw_evidence_preserved".to_string()),
                message: "Raw evidence must be preserved when aggregation is partial or failed"
                    .to_string(),
            });
        }

        if status != "ok"
            && manifest
                .states
                .iter()
                .all(|state| state.raw_artifact_paths.is_empty())
        {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("states.raw_artifact_paths".to_string()),
                message: "Aggregation failure must keep at least one raw artifact reference"
                    .to_string(),
            });
        }

        result
    }

    fn validate_relative_artifact_path(field: &str, path: &str) -> ValidationResult {
        let mut result = ValidationResult::ok();

        if path.trim().is_empty() {
            return result.with_error(ValidationError {
                line: None,
                field: Some(field.to_string()),
                message: "Path cannot be empty".to_string(),
            });
        }

        if path.starts_with('/') || path.contains(':') {
            result = result.with_error(ValidationError {
                line: None,
                field: Some(field.to_string()),
                message: format!("Path must be relative: {path}"),
            });
        }

        if path.contains("..") {
            result = result.with_error(ValidationError {
                line: None,
                field: Some(field.to_string()),
                message: format!("Path traversal detected: {path}"),
            });
        }

        result
    }

    /// Validate a startup matrix bundle directory, including referenced files.
    pub fn validate_startup_matrix_bundle_dir(&self, dir: &Path) -> ValidationResult {
        let manifest_path = dir.join(STARTUP_MATRIX_MANIFEST);
        let content = match fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                return ValidationResult::ok().with_error(ValidationError {
                    line: None,
                    field: Some(STARTUP_MATRIX_MANIFEST.to_string()),
                    message: format!("Failed to read startup matrix manifest: {e}"),
                });
            }
        };

        let manifest = match Self::parse_startup_matrix_manifest(&content) {
            Ok(manifest) => manifest,
            Err(error) => return ValidationResult::ok().with_error(error),
        };

        let mut result = self.validate_startup_matrix_manifest(&manifest);
        for state in &manifest.states {
            for path in [
                &state.command_log_path,
                &state.timing_summary_path,
                &state.syscall_summary_path,
                &state.rss_summary_path,
            ] {
                result = result.merge(Self::validate_bundle_file_exists(dir, path));
            }

            for path in &state.raw_artifact_paths {
                result = result.merge(Self::validate_bundle_file_exists(dir, path));
            }
        }

        result
    }

    /// Validate a performance evidence manifest file.
    pub fn validate_perf_evidence_manifest_file(&self, path: &Path) -> ValidationResult {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return ValidationResult::ok().with_error(ValidationError {
                    line: None,
                    field: None,
                    message: format!("Failed to read performance evidence manifest: {e}"),
                });
            }
        };

        self.validate_perf_evidence_manifest_content(&content)
    }

    /// Validate performance evidence manifest JSON content.
    #[allow(clippy::unused_self)]
    pub fn validate_perf_evidence_manifest_content(&self, content: &str) -> ValidationResult {
        let manifest = match Self::parse_perf_evidence_manifest(content) {
            Ok(manifest) => manifest,
            Err(error) => return ValidationResult::ok().with_error(error),
        };

        Self::validate_perf_evidence_manifest(&manifest)
    }

    fn parse_perf_evidence_manifest(
        content: &str,
    ) -> Result<PerfEvidenceManifest, ValidationError> {
        serde_json::from_str(content).map_err(|e| ValidationError {
            line: None,
            field: None,
            message: format!("Invalid performance evidence manifest JSON: {e}"),
        })
    }

    fn validate_perf_evidence_manifest(manifest: &PerfEvidenceManifest) -> ValidationResult {
        let mut result = ValidationResult::ok();

        if manifest.schema_version != "obr.perf-evidence.v1" {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("schema_version".to_string()),
                message: format!(
                    "Expected obr.perf-evidence.v1, got {}",
                    manifest.schema_version
                ),
            });
        }

        result = result.merge(Self::validate_rfc3339_field(
            "generated_at",
            &manifest.generated_at,
        ));
        if let Some(valid_until) = manifest.valid_until.as_deref() {
            result = result.merge(Self::validate_rfc3339_field("valid_until", valid_until));
            if let Ok(expires_at) = DateTime::parse_from_rfc3339(valid_until)
                && expires_at.with_timezone(&Utc) < Utc::now()
            {
                result = result.with_error(ValidationError {
                    line: None,
                    field: Some("valid_until".to_string()),
                    message: format!("Performance evidence artifact is stale: {valid_until}"),
                });
            }
        }

        result = result.merge(Self::validate_non_empty_field(
            "command.label",
            &manifest.command.label,
        ));
        result = result.merge(Self::validate_non_empty_field(
            "dataset.name",
            &manifest.dataset.name,
        ));
        result = result.merge(Self::validate_non_empty_field(
            "git.revision",
            &manifest.git.revision,
        ));
        result = result.merge(Self::validate_non_empty_field(
            "binary.path",
            &manifest.binary.path,
        ));
        result = result.merge(Self::validate_non_empty_field(
            "environment.os",
            &manifest.environment.os,
        ));

        for env in &manifest.environment.env {
            result = result.merge(Self::validate_non_empty_field(
                "environment.env.name",
                &env.name,
            ));
        }

        result = result.merge(Self::validate_perf_timing(&manifest.timing));
        result = result.merge(Self::validate_perf_resources(&manifest.resources));
        result = result.merge(Self::validate_perf_golden(&manifest.golden));
        result = result.merge(Self::validate_relative_artifact_path(
            "isomorphism_note_path",
            &manifest.isomorphism_note_path,
        ));
        result = result.merge(Self::validate_perf_policy(manifest));

        for path in &manifest.raw_artifact_paths {
            result = result.merge(Self::validate_relative_artifact_path(
                "raw_artifact_paths",
                path,
            ));
        }

        result
    }

    fn validate_perf_timing(timing: &PerfEvidenceTiming) -> ValidationResult {
        let mut result = ValidationResult::ok();

        if timing.sample_count == 0 {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("timing.sample_count".to_string()),
                message: "Timing sample count must be greater than zero".to_string(),
            });
        }

        for (field, value) in [
            ("timing.min_ms", timing.min_ms),
            ("timing.p50_ms", timing.p50_ms),
            ("timing.p95_ms", timing.p95_ms),
            ("timing.p99_ms", timing.p99_ms),
            ("timing.max_ms", timing.max_ms),
        ] {
            if !value.is_finite() || value < 0.0 {
                result = result.with_error(ValidationError {
                    line: None,
                    field: Some(field.to_string()),
                    message: format!("Timing value must be finite and non-negative: {value}"),
                });
            }
        }

        if !(timing.min_ms <= timing.p50_ms
            && timing.p50_ms <= timing.p95_ms
            && timing.p95_ms <= timing.p99_ms
            && timing.p99_ms <= timing.max_ms)
        {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("timing".to_string()),
                message: "Timing quantiles must satisfy min <= p50 <= p95 <= p99 <= max"
                    .to_string(),
            });
        }

        result = result.merge(Self::validate_relative_artifact_path(
            "timing.summary_path",
            &timing.summary_path,
        ));
        result = result.merge(Self::validate_relative_artifact_path(
            "timing.raw_samples_path",
            &timing.raw_samples_path,
        ));

        result
    }

    fn validate_perf_resources(resources: &PerfEvidenceResources) -> ValidationResult {
        let mut result = ValidationResult::ok();
        for (field, path) in [
            ("resources.syscall_summary_path", &resources.syscalls),
            ("resources.io_summary_path", &resources.io),
            ("resources.rss_summary_path", &resources.rss),
        ] {
            result = result.merge(Self::validate_relative_artifact_path(field, path));
        }
        result
    }

    fn validate_perf_golden(golden: &PerfEvidenceGolden) -> ValidationResult {
        let mut result = ValidationResult::ok();

        result = result.merge(Self::validate_sha256_field(
            "golden.stdout_sha256",
            &golden.stdout_sha256,
        ));
        if let Some(stderr_sha256) = golden.stderr_sha256.as_deref() {
            result = result.merge(Self::validate_sha256_field(
                "golden.stderr_sha256",
                stderr_sha256,
            ));
        }

        result = result.merge(Self::validate_relative_artifact_path(
            "golden.checksums_path",
            &golden.checksums_path,
        ));
        result = result.merge(Self::validate_relative_artifact_path(
            "golden.stdout_path",
            &golden.stdout_path,
        ));
        if let Some(stderr_path) = golden.stderr_path.as_deref() {
            result = result.merge(Self::validate_relative_artifact_path(
                "golden.stderr_path",
                stderr_path,
            ));
        }

        result
    }

    fn validate_perf_policy(manifest: &PerfEvidenceManifest) -> ValidationResult {
        let mut result = ValidationResult::ok();
        let mode = manifest.policy.mode.as_str();
        let status = manifest.comparison.status.as_str();

        if !matches!(mode, "advisory" | "enforcing") {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("policy.mode".to_string()),
                message: format!("Must be 'advisory' or 'enforcing', got: {mode}"),
            });
        }

        if !matches!(status, "pass" | "advisory" | "failed" | "not_compared") {
            result = result.with_error(ValidationError {
                line: None,
                field: Some("comparison.status".to_string()),
                message: format!(
                    "Must be 'pass', 'advisory', 'failed', or 'not_compared', got: {status}"
                ),
            });
        }

        result = result.merge(Self::validate_non_empty_field(
            "comparison.decision_reason",
            &manifest.comparison.decision_reason,
        ));

        if mode == "enforcing" {
            if manifest.policy.baseline_manifest_path.is_none() {
                result = result.with_error(ValidationError {
                    line: None,
                    field: Some("policy.baseline_manifest_path".to_string()),
                    message: "Enforcing mode requires a baseline manifest path".to_string(),
                });
            }

            if manifest.comparison.baseline_manifest_path.is_none() {
                result = result.with_error(ValidationError {
                    line: None,
                    field: Some("comparison.baseline_manifest_path".to_string()),
                    message: "Enforcing mode requires a compared baseline manifest path"
                        .to_string(),
                });
            }

            if status != "pass" {
                result = result.with_error(ValidationError {
                    line: None,
                    field: Some("comparison.status".to_string()),
                    message: format!("Enforcing mode requires comparison pass, got: {status}"),
                });
            }
        } else if status == "failed" {
            result = result.with_warning(
                "Advisory performance evidence comparison failed without blocking".to_string(),
            );
        }

        if manifest.policy.output_hash_must_match
            && manifest.comparison.stdout_hash_match == Some(false)
        {
            if mode == "enforcing" {
                result = result.with_error(ValidationError {
                    line: None,
                    field: Some("comparison.stdout_hash_match".to_string()),
                    message: "Output hash mismatch under enforcing policy".to_string(),
                });
            } else {
                result = result.with_warning(
                    "Output hash mismatch recorded under advisory policy".to_string(),
                );
            }
        }

        for (field, path) in [
            (
                "policy.baseline_manifest_path",
                manifest.policy.baseline_manifest_path.as_deref(),
            ),
            (
                "comparison.baseline_manifest_path",
                manifest.comparison.baseline_manifest_path.as_deref(),
            ),
        ] {
            if let Some(path) = path {
                result = result.merge(Self::validate_relative_artifact_path(field, path));
            }
        }

        result
    }

    /// Validate a performance evidence bundle directory, including referenced files.
    pub fn validate_perf_evidence_bundle_dir(&self, dir: &Path) -> ValidationResult {
        let manifest_path = dir.join(PERF_EVIDENCE_MANIFEST);
        let content = match fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                return ValidationResult::ok().with_error(ValidationError {
                    line: None,
                    field: Some(PERF_EVIDENCE_MANIFEST.to_string()),
                    message: format!("Failed to read performance evidence manifest: {e}"),
                });
            }
        };

        let manifest = match Self::parse_perf_evidence_manifest(&content) {
            Ok(manifest) => manifest,
            Err(error) => return ValidationResult::ok().with_error(error),
        };

        let mut result = self.validate_perf_evidence_manifest_content(&content);
        for path in [
            &manifest.timing.summary_path,
            &manifest.timing.raw_samples_path,
            &manifest.resources.syscalls,
            &manifest.resources.io,
            &manifest.resources.rss,
            &manifest.golden.checksums_path,
            &manifest.golden.stdout_path,
            &manifest.isomorphism_note_path,
        ] {
            result = result.merge(Self::validate_perf_bundle_file_exists(dir, path));
        }

        if let Some(stderr_path) = manifest.golden.stderr_path.as_deref() {
            result = result.merge(Self::validate_perf_bundle_file_exists(dir, stderr_path));
        }

        for path in &manifest.raw_artifact_paths {
            result = result.merge(Self::validate_perf_bundle_file_exists(dir, path));
        }

        if let Some(baseline_path) = manifest.policy.baseline_manifest_path.as_deref() {
            result = result.merge(Self::validate_perf_bundle_file_exists(dir, baseline_path));
        }

        result
    }

    fn validate_bundle_file_exists(dir: &Path, relative_path: &str) -> ValidationResult {
        let path_result = Self::validate_relative_artifact_path("artifact_path", relative_path);
        if !path_result.valid {
            return path_result;
        }

        if !dir.join(relative_path).is_file() {
            return ValidationResult::ok().with_error(ValidationError {
                line: None,
                field: Some(relative_path.to_string()),
                message: "Referenced startup matrix artifact is missing".to_string(),
            });
        }

        ValidationResult::ok()
    }

    fn validate_perf_bundle_file_exists(dir: &Path, relative_path: &str) -> ValidationResult {
        let path_result = Self::validate_relative_artifact_path("artifact_path", relative_path);
        if !path_result.valid {
            return path_result;
        }

        if !dir.join(relative_path).is_file() {
            return ValidationResult::ok().with_error(ValidationError {
                line: None,
                field: Some(relative_path.to_string()),
                message: "Referenced performance evidence artifact is missing".to_string(),
            });
        }

        ValidationResult::ok()
    }

    fn validate_rfc3339_field(field: &str, value: &str) -> ValidationResult {
        if DateTime::parse_from_rfc3339(value).is_err() {
            return ValidationResult::ok().with_error(ValidationError {
                line: None,
                field: Some(field.to_string()),
                message: format!("Invalid RFC3339 timestamp: {value}"),
            });
        }

        ValidationResult::ok()
    }

    fn validate_non_empty_field(field: &str, value: &str) -> ValidationResult {
        if value.trim().is_empty() {
            return ValidationResult::ok().with_error(ValidationError {
                line: None,
                field: Some(field.to_string()),
                message: "Field cannot be empty".to_string(),
            });
        }

        ValidationResult::ok()
    }

    fn validate_sha256_field(field: &str, value: &str) -> ValidationResult {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return ValidationResult::ok().with_error(ValidationError {
                line: None,
                field: Some(field.to_string()),
                message: format!("Expected lowercase SHA-256 hex digest, got: {value}"),
            });
        }

        ValidationResult::ok()
    }

    /// Validate an entire artifact directory
    pub fn validate_artifact_dir(&self, dir: &Path) -> ValidationResult {
        let mut result = ValidationResult::ok();

        // Check events.jsonl
        let events_path = dir.join("events.jsonl");
        if events_path.exists() {
            result = result.merge(self.validate_events_file(&events_path));
        }

        // Check summary.json
        let summary_path = dir.join("summary.json");
        if summary_path.exists() {
            result = result.merge(self.validate_summary_file(&summary_path));
        }

        // Check all snapshot files
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json") {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.contains("snapshot") {
                        result = result.merge(self.validate_snapshot_file(&path));
                    }
                }
            }
        }

        if dir.join(STARTUP_MATRIX_MANIFEST).exists() {
            result = result.merge(self.validate_startup_matrix_bundle_dir(dir));
        }

        if dir.join(PERF_EVIDENCE_MANIFEST).exists() {
            result = result.merge(self.validate_perf_evidence_bundle_dir(dir));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn valid_event_passes() {
        let validator = ArtifactValidator::new();
        let content = r#"{"timestamp":"2026-01-17T12:34:56.000Z","event_type":"command","label":"init","binary":"obr","args":["init"],"cwd":"/tmp/test","exit_code":0,"success":true,"duration_ms":42,"stdout_len":64,"stderr_len":0}"#;
        let result = validator.validate_events_content(content);
        assert!(result.valid, "Errors: {:?}", result.errors);
    }

    #[test]
    fn invalid_timestamp_fails() {
        let validator = ArtifactValidator::new();
        let content = r#"{"timestamp":"not-a-date","event_type":"command","label":"init","binary":"obr","args":[],"cwd":"/tmp","exit_code":0,"success":true,"duration_ms":0,"stdout_len":0,"stderr_len":0}"#;
        let result = validator.validate_events_content(content);
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.field.as_deref() == Some("timestamp"))
        );
    }

    #[test]
    fn invalid_event_type_fails() {
        let validator = ArtifactValidator::new();
        let content = r#"{"timestamp":"2026-01-17T12:34:56.000Z","event_type":"invalid","label":"test","binary":"obr","args":[],"cwd":"/tmp","exit_code":0,"success":true,"duration_ms":0,"stdout_len":0,"stderr_len":0}"#;
        let result = validator.validate_events_content(content);
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.field.as_deref() == Some("event_type"))
        );
    }

    #[test]
    fn path_traversal_fails() {
        let validator = ArtifactValidator::new();
        let content = r#"{"timestamp":"2026-01-17T12:34:56.000Z","event_type":"command","label":"test","binary":"obr","args":[],"cwd":"/tmp","exit_code":0,"success":true,"duration_ms":0,"stdout_len":0,"stderr_len":0,"stdout_path":"../etc/passwd"}"#;
        let result = validator.validate_events_content(content);
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains("traversal"))
        );
    }

    #[test]
    fn valid_snapshot_passes() {
        let validator = ArtifactValidator::new();
        let content = r#"[{"path":".obr","size":0,"is_dir":true},{"path":".obr/obr.db","size":12288,"is_dir":false}]"#;
        let result = validator.validate_snapshot_content(content);
        assert!(result.valid, "Errors: {:?}", result.errors);
    }

    #[test]
    fn valid_summary_passes() {
        let validator = ArtifactValidator::new();
        let content = r#"{"suite":"e2e","test":"test_init","passed":true,"run_count":1,"timestamp":"2026-01-17T12:34:56.000Z"}"#;
        let result = validator.validate_summary_content(content);
        assert!(result.valid, "Errors: {:?}", result.errors);
    }

    #[test]
    fn empty_suite_fails() {
        let validator = ArtifactValidator::new();
        let content = r#"{"suite":"","test":"test","passed":true,"run_count":1,"timestamp":"2026-01-17T12:34:56.000Z"}"#;
        let result = validator.validate_summary_content(content);
        assert!(!result.valid);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.field.as_deref() == Some("suite"))
        );
    }

    fn startup_state_json(state: &str) -> String {
        format!(
            r#"{{"state":"{state}","command_log_path":"logs/{state}.log","timing_summary_path":"timing/{state}.json","syscall_summary_path":"syscalls/{state}.txt","rss_summary_path":"rss/{state}.json","raw_artifact_paths":["raw/{state}.trace"]}}"#
        )
    }

    fn startup_manifest_json(states: &[&str], status: &str, raw_preserved: bool) -> String {
        let states = states
            .iter()
            .map(|state| startup_state_json(state))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"schema_version":"obr.startup-matrix.v1","matrix_name":"smoke","generated_at":"2026-05-03T01:00:00Z","states":[{states}],"aggregation":{{"status":"{status}","raw_evidence_preserved":{raw_preserved}}}}}"#
        )
    }

    fn write_startup_bundle_files(dir: &Path, states: &[&str]) {
        for subdir in ["logs", "timing", "syscalls", "rss", "raw"] {
            fs::create_dir_all(dir.join(subdir)).expect("create startup matrix subdir");
        }

        for state in states {
            for path in [
                format!("logs/{state}.log"),
                format!("timing/{state}.json"),
                format!("syscalls/{state}.txt"),
                format!("rss/{state}.json"),
                format!("raw/{state}.trace"),
            ] {
                fs::write(dir.join(path), "startup matrix smoke artifact")
                    .expect("write startup matrix artifact");
            }
        }
    }

    #[test]
    fn valid_startup_matrix_manifest_passes() {
        let validator = ArtifactValidator::new();
        let content = startup_manifest_json(REQUIRED_STARTUP_STATES, "ok", true);
        let result = validator.validate_startup_matrix_manifest_content(&content);
        assert!(result.valid, "Errors: {:?}", result.errors);
    }

    #[test]
    fn startup_matrix_manifest_requires_all_states() {
        let validator = ArtifactValidator::new();
        let states_without_recovery_anomaly = REQUIRED_STARTUP_STATES
            .iter()
            .copied()
            .filter(|state| *state != "recovery_anomaly")
            .collect::<Vec<_>>();
        let content = startup_manifest_json(&states_without_recovery_anomaly, "ok", true);
        let result = validator.validate_startup_matrix_manifest_content(&content);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|error| {
            error
                .message
                .contains("Missing required startup state: recovery_anomaly")
        }));
    }

    #[test]
    fn startup_matrix_failed_aggregation_requires_raw_evidence() {
        let validator = ArtifactValidator::new();
        let content = startup_manifest_json(REQUIRED_STARTUP_STATES, "failed", false);
        let result = validator.validate_startup_matrix_manifest_content(&content);
        assert!(!result.valid);
        assert!(
            result.errors.iter().any(|error| {
                error.field.as_deref() == Some("aggregation.raw_evidence_preserved")
            })
        );
    }

    #[test]
    fn startup_matrix_bundle_validates_referenced_files() {
        let validator = ArtifactValidator::new();
        let dir = tempdir().expect("create tempdir");
        let manifest = startup_manifest_json(REQUIRED_STARTUP_STATES, "ok", true);
        fs::write(dir.path().join(STARTUP_MATRIX_MANIFEST), manifest)
            .expect("write startup matrix manifest");
        write_startup_bundle_files(dir.path(), REQUIRED_STARTUP_STATES);

        let result = validator.validate_startup_matrix_bundle_dir(dir.path());
        assert!(result.valid, "Errors: {:?}", result.errors);

        let incomplete_dir = tempdir().expect("create incomplete tempdir");
        let manifest = startup_manifest_json(REQUIRED_STARTUP_STATES, "ok", true);
        fs::write(
            incomplete_dir.path().join(STARTUP_MATRIX_MANIFEST),
            manifest,
        )
        .expect("write incomplete startup matrix manifest");
        let incomplete = validator.validate_startup_matrix_bundle_dir(incomplete_dir.path());
        assert!(!incomplete.valid);
        assert!(incomplete.errors.iter().any(|error| {
            error
                .message
                .contains("Referenced startup matrix artifact is missing")
        }));
    }

    fn perf_manifest_json(
        mode: &str,
        comparison_status: &str,
        stdout_hash_match: bool,
        valid_until: &str,
    ) -> String {
        format!(
            r#"{{
                "schema_version":"obr.perf-evidence.v1",
                "generated_at":"2026-05-03T02:00:00Z",
                "valid_until":"{valid_until}",
                "command":{{"label":"list_json","args":["list","--json"]}},
                "dataset":{{"name":"tiny-smoke","issue_count":3,"content_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}},
                "git":{{"revision":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","dirty":false}},
                "binary":{{"path":"target/debug/obr","version":"obr 0.2.5"}},
                "environment":{{"os":"linux","rustc":"rustc 1.91.0-nightly","env":[{{"name":"NO_COLOR","value_hash":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}}]}},
                "timing":{{"sample_count":3,"min_ms":1.0,"p50_ms":2.0,"p95_ms":3.0,"p99_ms":3.0,"max_ms":3.0,"summary_path":"timing/list.json","raw_samples_path":"timing/list-samples.jsonl"}},
                "resources":{{"syscall_summary_path":"syscalls/list.json","io_summary_path":"io/list.json","rss_summary_path":"rss/list.json"}},
                "golden":{{"stdout_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","stderr_sha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","checksums_path":"golden/checksums.txt","stdout_path":"golden/stdout","stderr_path":"golden/stderr"}},
                "isomorphism_note_path":"proof/isomorphism.md",
                "policy":{{"mode":"{mode}","baseline_manifest_path":"baseline/perf-evidence-manifest.json","latency_regression_budget_pct":5.0,"syscall_regression_budget_pct":10.0,"output_hash_must_match":true}},
                "comparison":{{"status":"{comparison_status}","baseline_manifest_path":"baseline/perf-evidence-manifest.json","p95_delta_pct":0.0,"stdout_hash_match":{stdout_hash_match},"syscall_delta_pct":0.0,"decision_reason":"self-baseline smoke comparison"}},
                "raw_artifact_paths":["raw/list-0.stdout","raw/list-0.stderr"]
            }}"#
        )
    }

    fn write_perf_bundle_files(dir: &Path) {
        for subdir in [
            "timing", "syscalls", "io", "rss", "golden", "proof", "baseline", "raw",
        ] {
            fs::create_dir_all(dir.join(subdir)).expect("create perf evidence subdir");
        }

        for path in [
            "timing/list.json",
            "timing/list-samples.jsonl",
            "syscalls/list.json",
            "io/list.json",
            "rss/list.json",
            "golden/checksums.txt",
            "golden/stdout",
            "golden/stderr",
            "proof/isomorphism.md",
            "baseline/perf-evidence-manifest.json",
            "raw/list-0.stdout",
            "raw/list-0.stderr",
        ] {
            fs::write(dir.join(path), "perf evidence smoke artifact")
                .expect("write perf evidence artifact");
        }
    }

    #[test]
    fn valid_perf_evidence_manifest_passes() {
        let validator = ArtifactValidator::new();
        let content = perf_manifest_json("enforcing", "pass", true, "2099-01-01T00:00:00Z");
        let result = validator.validate_perf_evidence_manifest_content(&content);
        assert!(result.valid, "Errors: {:?}", result.errors);
    }

    #[test]
    fn perf_evidence_manifest_rejects_stale_artifacts() {
        let validator = ArtifactValidator::new();
        let content = perf_manifest_json("enforcing", "pass", true, "2000-01-01T00:00:00Z");
        let result = validator.validate_perf_evidence_manifest_content(&content);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|error| {
            error.field.as_deref() == Some("valid_until") && error.message.contains("stale")
        }));
    }

    #[test]
    fn perf_evidence_enforcing_policy_requires_passing_comparison() {
        let validator = ArtifactValidator::new();
        let content = perf_manifest_json("enforcing", "advisory", true, "2099-01-01T00:00:00Z");
        let result = validator.validate_perf_evidence_manifest_content(&content);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|error| {
            error.field.as_deref() == Some("comparison.status")
                && error.message.contains("requires comparison pass")
        }));
    }

    #[test]
    fn perf_evidence_advisory_policy_warns_without_failing() {
        let validator = ArtifactValidator::new();
        let content = perf_manifest_json("advisory", "failed", false, "2099-01-01T00:00:00Z");
        let result = validator.validate_perf_evidence_manifest_content(&content);
        assert!(result.valid, "Errors: {:?}", result.errors);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn perf_evidence_bundle_validates_referenced_files() {
        let validator = ArtifactValidator::new();
        let dir = tempdir().expect("create tempdir");
        let manifest = perf_manifest_json("enforcing", "pass", true, "2099-01-01T00:00:00Z");
        fs::write(dir.path().join(PERF_EVIDENCE_MANIFEST), manifest)
            .expect("write perf evidence manifest");
        write_perf_bundle_files(dir.path());

        let result = validator.validate_perf_evidence_bundle_dir(dir.path());
        assert!(result.valid, "Errors: {:?}", result.errors);

        let incomplete_dir = tempdir().expect("create incomplete tempdir");
        let manifest = perf_manifest_json("enforcing", "pass", true, "2099-01-01T00:00:00Z");
        fs::write(incomplete_dir.path().join(PERF_EVIDENCE_MANIFEST), manifest)
            .expect("write incomplete perf evidence manifest");
        let incomplete = validator.validate_perf_evidence_bundle_dir(incomplete_dir.path());
        assert!(!incomplete.valid);
        assert!(incomplete.errors.iter().any(|error| {
            error
                .message
                .contains("Referenced performance evidence artifact is missing")
        }));
    }
}
