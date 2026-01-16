//! JSONL import/export for `beads_rust`.
//!
//! This module handles:
//! - Export: `SQLite` -> JSONL (for git tracking)
//! - Import: JSONL -> `SQLite` (for git clone/pull)
//! - Dirty tracking for incremental exports
//! - Collision detection during imports
//! - Path validation and allowlist enforcement

pub mod path;

pub use path::{
    ALLOWED_EXACT_NAMES, ALLOWED_EXTENSIONS, PathValidation, is_sync_path_allowed,
    require_valid_sync_path, validate_sync_path,
};

use crate::error::{BeadsError, Result};
use crate::model::Issue;
use crate::storage::SqliteStorage;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Configuration for JSONL export.
#[derive(Debug, Clone, Default)]
pub struct ExportConfig {
    /// Force export even if database is empty and JSONL has issues.
    pub force: bool,
    /// Whether this is an export to the default JSONL path (affects dirty flag clearing).
    pub is_default_path: bool,
    /// Error handling policy for export.
    pub error_policy: ExportErrorPolicy,
    /// Retention period for tombstones in days (None = keep forever).
    pub retention_days: Option<u64>,
}

/// Export error handling policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ExportErrorPolicy {
    /// Abort export on any error (default).
    #[default]
    Strict,
    /// Skip problematic records, export what we can.
    BestEffort,
    /// Export valid records, report failures.
    Partial,
    /// Only export core issues; non-core errors are tolerated.
    RequiredCore,
}

impl std::fmt::Display for ExportErrorPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Strict => "strict",
            Self::BestEffort => "best-effort",
            Self::Partial => "partial",
            Self::RequiredCore => "required-core",
        };
        write!(f, "{value}")
    }
}

impl std::str::FromStr for ExportErrorPolicy {
    type Err = String;

    fn from_str(input: &str) -> std::result::Result<Self, Self::Err> {
        match input.to_ascii_lowercase().as_str() {
            "strict" => Ok(Self::Strict),
            "best-effort" | "best_effort" | "best" => Ok(Self::BestEffort),
            "partial" => Ok(Self::Partial),
            "required-core" | "required_core" | "core" => Ok(Self::RequiredCore),
            other => Err(format!(
                "Invalid error policy: {other}. Must be one of: strict, best-effort, partial, required-core"
            )),
        }
    }
}

/// Export entity types for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportEntityType {
    Issue,
    Dependency,
    Label,
    Comment,
}

/// Export error record.
#[derive(Debug, Clone, Serialize)]
pub struct ExportError {
    pub entity_type: ExportEntityType,
    pub entity_id: String,
    pub message: String,
}

impl ExportError {
    fn new(
        entity_type: ExportEntityType,
        entity_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            entity_type,
            entity_id: entity_id.into(),
            message: message.into(),
        }
    }

    #[must_use]
    pub fn summary(&self) -> String {
        let id = if self.entity_id.is_empty() {
            "<unknown>"
        } else {
            self.entity_id.as_str()
        };
        format!("{:?} {id}: {}", self.entity_type, self.message)
    }
}

/// Export report with error details and counts.
#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    pub issues_exported: usize,
    pub dependencies_exported: usize,
    pub labels_exported: usize,
    pub comments_exported: usize,
    pub errors: Vec<ExportError>,
    pub policy_used: ExportErrorPolicy,
}

impl ExportReport {
    const fn new(policy: ExportErrorPolicy) -> Self {
        Self {
            issues_exported: 0,
            dependencies_exported: 0,
            labels_exported: 0,
            comments_exported: 0,
            errors: Vec::new(),
            policy_used: policy,
        }
    }

    /// True if any errors were recorded.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Success rate for exported entities.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn success_rate(&self) -> f64 {
        let total = self.issues_exported
            + self.dependencies_exported
            + self.labels_exported
            + self.comments_exported;
        let failed = self.errors.len();
        if total + failed == 0 {
            1.0
        } else {
            total as f64 / (total + failed) as f64
        }
    }
}

struct ExportContext {
    policy: ExportErrorPolicy,
    errors: Vec<ExportError>,
}

impl ExportContext {
    const fn new(policy: ExportErrorPolicy) -> Self {
        Self {
            policy,
            errors: Vec::new(),
        }
    }

    fn handle_error(&mut self, err: ExportError) -> Result<()> {
        match self.policy {
            ExportErrorPolicy::Strict => Err(BeadsError::Config(format!(
                "Export error: {}",
                err.summary()
            ))),
            ExportErrorPolicy::BestEffort | ExportErrorPolicy::Partial => {
                self.errors.push(err);
                Ok(())
            }
            ExportErrorPolicy::RequiredCore => {
                if err.entity_type == ExportEntityType::Issue {
                    Err(BeadsError::Config(format!(
                        "Export error: {}",
                        err.summary()
                    )))
                } else {
                    self.errors.push(err);
                    Ok(())
                }
            }
        }
    }
}

/// Result of a JSONL export operation.
#[derive(Debug, Clone)]
pub struct ExportResult {
    /// Number of issues exported.
    pub exported_count: usize,
    /// IDs of exported issues.
    pub exported_ids: Vec<String>,
    /// SHA256 hash of the exported JSONL content.
    pub content_hash: String,
    /// Output file path (None if stdout).
    pub output_path: Option<String>,
    /// Per-issue content hashes (`issue_id`, `content_hash`) for incremental export tracking.
    pub issue_hashes: Vec<(String, String)>,
}

/// Configuration for JSONL import.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct ImportConfig {
    /// Skip prefix validation when importing.
    pub skip_prefix_validation: bool,
    /// Rewrite IDs and references on prefix mismatch.
    pub rename_on_import: bool,
    /// Clear duplicate external refs instead of erroring.
    pub clear_duplicate_external_refs: bool,
    /// How to handle orphaned issues during import.
    pub orphan_mode: OrphanMode,
    /// Force upsert even if timestamps are equal or older.
    pub force_upsert: bool,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            skip_prefix_validation: false,
            rename_on_import: false,
            clear_duplicate_external_refs: false,
            orphan_mode: OrphanMode::Strict,
            force_upsert: false,
        }
    }
}

/// Orphan handling behavior for import.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanMode {
    /// Fail if any issue references a missing parent.
    Strict,
    /// Attempt to resurrect missing parents if found.
    Resurrect,
    /// Skip orphaned issues.
    Skip,
    /// Allow orphans (no parent validation).
    Allow,
}

/// Result of a JSONL import.
#[derive(Debug, Clone, Default)]
pub struct ImportResult {
    /// Number of issues imported (created or updated).
    pub imported_count: usize,
    /// Number of issues skipped.
    pub skipped_count: usize,
    /// Number of tombstones skipped.
    pub tombstone_skipped: usize,
    /// Conflict markers detected (if any).
    pub conflict_markers: Vec<ConflictMarker>,
}

/// Conflict marker kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictMarkerType {
    Start,
    Separator,
    End,
}

/// A detected merge conflict marker within an import file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictMarker {
    pub path: PathBuf,
    pub line: usize,
    pub marker_type: ConflictMarkerType,
    pub branch: Option<String>,
}

const CONFLICT_START: &str = "<<<<<<<";
const CONFLICT_SEPARATOR: &str = "=======";
const CONFLICT_END: &str = ">>>>>>>";

/// Scan a file for merge conflict markers.
///
/// # Errors
///
/// Returns an error if the file cannot be read.
pub fn scan_conflict_markers(path: &Path) -> Result<Vec<ConflictMarker>> {
    let file = File::open(path)?;
    let reader = BufReader::with_capacity(2 * 1024 * 1024, file);
    let mut markers = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        if let Some((marker_type, branch)) = detect_conflict_marker(&line) {
            markers.push(ConflictMarker {
                path: path.to_path_buf(),
                line: line_num + 1,
                marker_type,
                branch,
            });
        }
    }

    Ok(markers)
}

fn detect_conflict_marker(line: &str) -> Option<(ConflictMarkerType, Option<String>)> {
    if let Some(branch) = line.strip_prefix(CONFLICT_START) {
        return Some((ConflictMarkerType::Start, Some(branch.trim().to_string())));
    }
    if line.starts_with(CONFLICT_SEPARATOR) {
        return Some((ConflictMarkerType::Separator, None));
    }
    if let Some(branch) = line.strip_prefix(CONFLICT_END) {
        return Some((ConflictMarkerType::End, Some(branch.trim().to_string())));
    }
    None
}

/// Fail if a file contains merge conflict markers.
///
/// # Errors
///
/// Returns a config error describing the first few markers found.
pub fn ensure_no_conflict_markers(path: &Path) -> Result<()> {
    let markers = scan_conflict_markers(path)?;
    if markers.is_empty() {
        return Ok(());
    }

    let mut preview = String::new();
    for marker in markers.iter().take(5) {
        let _ = writeln!(
            preview,
            "{}:{} {:?}{}",
            marker.path.display(),
            marker.line,
            marker.marker_type,
            marker
                .branch
                .as_ref()
                .map_or(String::new(), |b| format!(" ({b})"))
        );
    }

    Err(BeadsError::Config(format!(
        "Merge conflict markers detected in {}.\n{}Resolve conflicts before importing.",
        path.display(),
        preview
    )))
}

/// Count issues in an existing JSONL file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or contains invalid JSON.
pub fn count_issues_in_jsonl(path: &Path) -> Result<usize> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(BeadsError::Io(e)),
    };

    let reader = BufReader::new(file);
    let mut count = 0;

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // Validate JSON without fully deserializing
        if serde_json::from_str::<serde_json::Value>(&line).is_err() {
            return Err(BeadsError::Config(format!(
                "Invalid JSON at line {}: {}",
                line_num + 1,
                line.chars().take(50).collect::<String>()
            )));
        }
        count += 1;
    }

    Ok(count)
}

/// Get issue IDs from an existing JSONL file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or contains invalid JSON.
pub fn get_issue_ids_from_jsonl(path: &Path) -> Result<HashSet<String>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(e) => return Err(BeadsError::Io(e)),
    };

    let reader = BufReader::new(file);
    let mut ids = HashSet::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // Parse just enough to get the ID
        let value: serde_json::Value = serde_json::from_str(&line).map_err(|e| {
            BeadsError::Config(format!("Invalid JSON at line {}: {}", line_num + 1, e))
        })?;

        if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
            ids.insert(id.to_string());
        }
    }

    Ok(ids)
}

/// Export issues from `SQLite` to JSONL format.
///
/// This implements the classic beads export semantics:
/// - Include tombstones (for sync propagation)
/// - Exclude ephemerals/wisps
/// - Sort by ID for deterministic output
/// - Populate dependencies and labels for each issue
/// - Atomic write (temp file -> rename)
/// - Safety guard against empty DB overwriting non-empty JSONL
///
/// # Errors
///
/// Returns an error if:
/// - Database read fails
/// - Safety guard is violated (empty DB, non-empty JSONL, no force)
/// - File write fails
#[allow(clippy::too_many_lines)]
pub fn export_to_jsonl(
    storage: &SqliteStorage,
    output_path: &Path,
    config: &ExportConfig,
) -> Result<ExportResult> {
    let (result, _report) = export_to_jsonl_with_policy(storage, output_path, config)?;
    Ok(result)
}

/// Export issues with configurable error policy, returning a report.
///
/// # Errors
///
/// Returns an error if:
/// - Database queries fail and the policy requires strict handling
/// - Safety guards are violated (empty/stale export without `force`)
/// - File I/O fails
#[allow(clippy::too_many_lines)]
pub fn export_to_jsonl_with_policy(
    storage: &SqliteStorage,
    output_path: &Path,
    config: &ExportConfig,
) -> Result<(ExportResult, ExportReport)> {
    // Get all issues for export (sorted by ID, excludes ephemerals/wisps)
    let mut issues = storage.get_all_issues_for_export()?;

    // Safety check: prevent exporting empty database over non-empty JSONL
    if issues.is_empty() && !config.force {
        let existing_count = count_issues_in_jsonl(output_path)?;
        if existing_count > 0 {
            return Err(BeadsError::Config(format!(
                "Refusing to export empty database over non-empty JSONL file.\n\
                 Database has 0 issues, JSONL has {existing_count} issues.\n\
                 This would result in data loss!\n\
                 Hint: Use --force to override this safety check."
            )));
        }
    }

    // Safety check: prevent exporting stale database that would lose issues
    if !config.force && output_path.exists() {
        let jsonl_ids = get_issue_ids_from_jsonl(output_path)?;
        if !jsonl_ids.is_empty() {
            let db_ids: HashSet<String> = issues.iter().map(|i| i.id.clone()).collect();
            let missing: Vec<_> = jsonl_ids.difference(&db_ids).collect();

            if !missing.is_empty() {
                let mut missing_list = missing.into_iter().cloned().collect::<Vec<_>>();
                missing_list.sort();
                let display_count = missing_list.len().min(10);
                let preview: Vec<_> = missing_list.iter().take(display_count).collect();
                let more = if missing_list.len() > 10 {
                    format!(" ... and {} more", missing_list.len() - 10)
                } else {
                    String::new()
                };

                return Err(BeadsError::Config(format!(
                    "Refusing to export stale database that would lose issues.\n\
                     Database has {} issues, JSONL has {} issues.\n\
                     Export would lose {} issue(s): {}{}\n\
                     Hint: Run import first, or use --force to override.",
                    issues.len(),
                    jsonl_ids.len(),
                    missing_list.len(),
                    preview
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    more
                )));
            }
        }
    }

    let mut ctx = ExportContext::new(config.error_policy);
    let mut report = ExportReport::new(config.error_policy);

    // Populate dependencies and labels for all issues (batch queries to avoid N+1)
    let all_deps = match storage.get_all_dependency_records() {
        Ok(map) => Some(map),
        Err(err) => {
            ctx.handle_error(ExportError::new(
                ExportEntityType::Dependency,
                "all",
                err.to_string(),
            ))?;
            None
        }
    };
    let all_labels = match storage.get_all_labels() {
        Ok(map) => Some(map),
        Err(err) => {
            ctx.handle_error(ExportError::new(
                ExportEntityType::Label,
                "all",
                err.to_string(),
            ))?;
            None
        }
    };

    for issue in &mut issues {
        if let Some(deps) = all_deps.as_ref().and_then(|map| map.get(&issue.id)) {
            issue.dependencies = deps.clone();
        } else {
            issue.dependencies.clear();
        }
        if let Some(labels) = all_labels.as_ref().and_then(|map| map.get(&issue.id)) {
            issue.labels = labels.clone();
        } else {
            issue.labels.clear();
        }
        match storage.get_comments(&issue.id) {
            Ok(comments) => {
                issue.comments = comments;
            }
            Err(err) => {
                ctx.handle_error(ExportError::new(
                    ExportEntityType::Comment,
                    issue.id.clone(),
                    err.to_string(),
                ))?;
                issue.comments.clear();
            }
        }
    }

    // Write to temp file for atomic rename
    let parent_dir = output_path.parent().ok_or_else(|| {
        BeadsError::Config(format!("Invalid output path: {}", output_path.display()))
    })?;

    // Ensure parent directory exists
    fs::create_dir_all(parent_dir)?;

    let temp_path = output_path.with_extension("jsonl.tmp");
    let temp_file = File::create(&temp_path)?;
    let mut writer = BufWriter::new(temp_file);

    // Write JSONL and compute hash
    let mut hasher = Sha256::new();
    let mut exported_ids = Vec::new();
    let mut issue_hashes = Vec::new();

    for issue in &issues {
        // Skip expired tombstones
        if issue.is_expired_tombstone(config.retention_days) {
            continue;
        }

        let json = match serde_json::to_string(issue) {
            Ok(json) => json,
            Err(err) => {
                ctx.handle_error(ExportError::new(
                    ExportEntityType::Issue,
                    issue.id.clone(),
                    err.to_string(),
                ))?;
                continue;
            }
        };

        if let Err(err) = writeln!(writer, "{json}") {
            ctx.handle_error(ExportError::new(
                ExportEntityType::Issue,
                issue.id.clone(),
                err.to_string(),
            ))?;
            continue;
        }

        hasher.update(json.as_bytes());
        hasher.update(b"\n");

        exported_ids.push(issue.id.clone());
        issue_hashes.push((
            issue.id.clone(),
            issue
                .content_hash
                .clone()
                .unwrap_or_else(|| crate::util::content_hash(issue)),
        ));
        report.issues_exported += 1;
        report.dependencies_exported += issue.dependencies.len();
        report.labels_exported += issue.labels.len();
        report.comments_exported += issue.comments.len();
    }

    // Flush and sync
    writer.flush()?;
    writer
        .into_inner()
        .map_err(|e| BeadsError::Io(e.into_error()))?
        .sync_all()?;

    // Atomic rename
    fs::rename(&temp_path, output_path)?;

    // Set file permissions (0600)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = fs::set_permissions(output_path, perms);
    }

    // Compute final hash
    let content_hash = format!("{:x}", hasher.finalize());

    // Verify export integrity
    let actual_count = count_issues_in_jsonl(output_path)?;
    if actual_count != exported_ids.len() {
        return Err(BeadsError::Config(format!(
            "Export verification failed: expected {} issues, JSONL has {} lines",
            exported_ids.len(),
            actual_count
        )));
    }

    let result = ExportResult {
        exported_count: exported_ids.len(),
        exported_ids,
        content_hash,
        output_path: Some(output_path.to_string_lossy().to_string()),
        issue_hashes,
    };

    report.errors = ctx.errors;

    Ok((result, report))
}

/// Export issues to a writer (e.g., stdout).
///
/// # Errors
///
/// Returns an error if serialization or writing fails.
pub fn export_to_writer<W: Write>(storage: &SqliteStorage, writer: &mut W) -> Result<ExportResult> {
    let (result, _report) =
        export_to_writer_with_policy(storage, writer, ExportErrorPolicy::Strict)?;
    Ok(result)
}

/// Export issues to a writer with configurable error policy.
///
/// # Errors
///
/// Returns an error if serialization or writing fails under a strict policy.
pub fn export_to_writer_with_policy<W: Write>(
    storage: &SqliteStorage,
    writer: &mut W,
    policy: ExportErrorPolicy,
) -> Result<(ExportResult, ExportReport)> {
    let mut issues = storage.get_all_issues_for_export()?;

    // Populate dependencies and labels
    let mut ctx = ExportContext::new(policy);
    let mut report = ExportReport::new(policy);
    let all_deps = match storage.get_all_dependency_records() {
        Ok(map) => Some(map),
        Err(err) => {
            ctx.handle_error(ExportError::new(
                ExportEntityType::Dependency,
                "all",
                err.to_string(),
            ))?;
            None
        }
    };
    let all_labels = match storage.get_all_labels() {
        Ok(map) => Some(map),
        Err(err) => {
            ctx.handle_error(ExportError::new(
                ExportEntityType::Label,
                "all",
                err.to_string(),
            ))?;
            None
        }
    };

    for issue in &mut issues {
        if let Some(deps) = all_deps.as_ref().and_then(|map| map.get(&issue.id)) {
            issue.dependencies = deps.clone();
        } else {
            issue.dependencies.clear();
        }
        if let Some(labels) = all_labels.as_ref().and_then(|map| map.get(&issue.id)) {
            issue.labels = labels.clone();
        } else {
            issue.labels.clear();
        }
        match storage.get_comments(&issue.id) {
            Ok(comments) => issue.comments = comments,
            Err(err) => {
                ctx.handle_error(ExportError::new(
                    ExportEntityType::Comment,
                    issue.id.clone(),
                    err.to_string(),
                ))?;
                issue.comments.clear();
            }
        }
    }

    let mut hasher = Sha256::new();
    let mut exported_ids = Vec::new();
    let mut issue_hashes = Vec::new();

    for issue in &issues {
        let json = match serde_json::to_string(issue) {
            Ok(json) => json,
            Err(err) => {
                ctx.handle_error(ExportError::new(
                    ExportEntityType::Issue,
                    issue.id.clone(),
                    err.to_string(),
                ))?;
                continue;
            }
        };
        if let Err(err) = writeln!(writer, "{json}") {
            ctx.handle_error(ExportError::new(
                ExportEntityType::Issue,
                issue.id.clone(),
                err.to_string(),
            ))?;
            continue;
        }
        hasher.update(json.as_bytes());
        hasher.update(b"\n");

        exported_ids.push(issue.id.clone());
        issue_hashes.push((
            issue.id.clone(),
            issue
                .content_hash
                .clone()
                .unwrap_or_else(|| crate::util::content_hash(issue)),
        ));
        report.issues_exported += 1;
        report.dependencies_exported += issue.dependencies.len();
        report.labels_exported += issue.labels.len();
        report.comments_exported += issue.comments.len();
    }

    let content_hash = format!("{:x}", hasher.finalize());

    let result = ExportResult {
        exported_count: exported_ids.len(),
        exported_ids,
        content_hash,
        output_path: None,
        issue_hashes,
    };

    report.errors = ctx.errors;

    Ok((result, report))
}

/// Metadata key for the JSONL content hash.
pub const METADATA_JSONL_CONTENT_HASH: &str = "jsonl_content_hash";
/// Metadata key for the last export time.
pub const METADATA_LAST_EXPORT_TIME: &str = "last_export_time";
/// Metadata key for the last import time.
pub const METADATA_LAST_IMPORT_TIME: &str = "last_import_time";

/// Finalize an export by updating metadata, clearing dirty flags, and recording export hashes.
///
/// This should be called after a successful export to the default JSONL path.
/// It performs the following updates:
/// - Clears dirty flags for the exported issue IDs
/// - Records export hashes for each exported issue (for incremental export)
/// - Updates `jsonl_content_hash` metadata with the export hash
/// - Updates `last_export_time` metadata with the current timestamp
///
/// # Errors
///
/// Returns an error if database updates fail.
pub fn finalize_export(
    storage: &mut SqliteStorage,
    result: &ExportResult,
    issue_hashes: Option<&[(String, String)]>,
) -> Result<()> {
    use chrono::Utc;

    // Clear dirty flags for exported issues
    if !result.exported_ids.is_empty() {
        storage.clear_dirty_issues(&result.exported_ids)?;
    }

    // Record export hashes for each exported issue (for incremental export detection)
    if let Some(hashes) = issue_hashes {
        storage.set_export_hashes(hashes)?;
    }

    // Update metadata
    storage.set_metadata(METADATA_JSONL_CONTENT_HASH, &result.content_hash)?;
    storage.set_metadata(METADATA_LAST_EXPORT_TIME, &Utc::now().to_rfc3339())?;

    Ok(())
}

/// Read all issues from a JSONL file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or contains invalid JSON.
pub fn read_issues_from_jsonl(path: &Path) -> Result<Vec<Issue>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut issues = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let issue: Issue = serde_json::from_str(&line).map_err(|e| {
            BeadsError::Config(format!("Invalid JSON at line {}: {}", line_num + 1, e))
        })?;
        issues.push(issue);
    }

    Ok(issues)
}

// ===== 4-Phase Collision Detection =====

/// Match type from collision detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchType {
    /// Matched by external reference (e.g., JIRA-123).
    ExternalRef,
    /// Matched by content hash (deduplication).
    ContentHash,
    /// Matched by ID.
    Id,
}

/// Result of collision detection.
#[derive(Debug, Clone)]
pub enum CollisionResult {
    /// No match found - issue is new.
    NewIssue,
    /// Matched an existing issue.
    Match {
        /// The existing issue ID.
        existing_id: String,
        /// How the match was determined.
        match_type: MatchType,
        /// Which phase found the match (1-3).
        phase: u8,
    },
}

/// Action to take after collision detection.
#[derive(Debug, Clone)]
pub enum CollisionAction {
    /// Insert as a new issue.
    Insert,
    /// Update the existing issue.
    Update { existing_id: String },
    /// Skip this issue (existing is newer or it's a tombstone).
    Skip { reason: String },
}

/// Detect collision for an incoming issue using the 4-phase algorithm.
///
/// Phases:
/// 1. External reference match
/// 2. Content hash match
/// 3. ID match
/// 4. No match (new issue)
fn detect_collision(
    incoming: &Issue,
    storage: &SqliteStorage,
    computed_hash: &str,
) -> Result<CollisionResult> {
    // Phase 1: External reference match
    if let Some(ref external_ref) = incoming.external_ref {
        if let Some(existing) = storage.find_by_external_ref(external_ref)? {
            return Ok(CollisionResult::Match {
                existing_id: existing.id,
                match_type: MatchType::ExternalRef,
                phase: 1,
            });
        }
    }

    // Phase 2: Content hash match
    if let Some(existing) = storage.find_by_content_hash(computed_hash)? {
        return Ok(CollisionResult::Match {
            existing_id: existing.id,
            match_type: MatchType::ContentHash,
            phase: 2,
        });
    }

    // Phase 3: ID match
    if storage.id_exists(&incoming.id)? {
        return Ok(CollisionResult::Match {
            existing_id: incoming.id.clone(),
            match_type: MatchType::Id,
            phase: 3,
        });
    }

    // Phase 4: No match
    Ok(CollisionResult::NewIssue)
}

/// Determine the action to take based on collision result.
fn determine_action(
    collision: &CollisionResult,
    incoming: &Issue,
    storage: &SqliteStorage,
    force_upsert: bool,
) -> Result<CollisionAction> {
    match collision {
        CollisionResult::NewIssue => Ok(CollisionAction::Insert),
        CollisionResult::Match { existing_id, .. } => {
            // Check for tombstone protection (even force doesn't override this)
            if storage.is_tombstone(existing_id)? {
                return Ok(CollisionAction::Skip {
                    reason: format!("Tombstone protection: {existing_id}"),
                });
            }

            // If force_upsert is enabled, always update (skip timestamp comparison)
            if force_upsert {
                return Ok(CollisionAction::Update {
                    existing_id: existing_id.clone(),
                });
            }

            // Get existing issue for timestamp comparison
            let existing =
                storage
                    .get_issue(existing_id)?
                    .ok_or_else(|| BeadsError::IssueNotFound {
                        id: existing_id.clone(),
                    })?;

            // Last-write-wins: compare updated_at
            match incoming.updated_at.cmp(&existing.updated_at) {
                std::cmp::Ordering::Greater => Ok(CollisionAction::Update {
                    existing_id: existing_id.clone(),
                }),
                std::cmp::Ordering::Equal => Ok(CollisionAction::Skip {
                    reason: format!("Equal timestamps: {existing_id}"),
                }),
                std::cmp::Ordering::Less => Ok(CollisionAction::Skip {
                    reason: format!("Existing is newer: {existing_id}"),
                }),
            }
        }
    }
}

/// Normalize an issue for import.
///
/// - Recomputes `content_hash`
/// - Sets ephemeral=true if ID contains "-wisp-"
/// - Applies defaults and repairs `closed_at` invariant
fn normalize_issue(issue: &mut Issue) {
    use crate::util::content_hash;

    // Recompute content hash
    issue.content_hash = Some(content_hash(issue));

    // Wisp detection: if ID contains "-wisp-", mark as ephemeral
    if issue.id.contains("-wisp-") {
        issue.ephemeral = true;
    }

    // Repair closed_at invariant: if status is closed/tombstone, ensure closed_at is set
    if matches!(
        issue.status,
        crate::model::Status::Closed | crate::model::Status::Tombstone
    ) && issue.closed_at.is_none()
    {
        issue.closed_at = Some(issue.updated_at);
    }

    // If status is not closed/tombstone, clear closed_at
    if !matches!(
        issue.status,
        crate::model::Status::Closed | crate::model::Status::Tombstone
    ) {
        issue.closed_at = None;
    }
}

/// Import issues from a JSONL file.
///
/// Implements classic bd import semantics:
/// 1. Conflict marker scan - abort if found
/// 2. Parse JSONL with 2MB buffer
/// 3. Normalize issues (recompute `content_hash`, set defaults)
/// 4. Prefix validation (optional)
/// 5. 4-phase collision detection
/// 6. Tombstone protection
/// 7. Orphan handling
/// 8. Create/update issues
/// 9. Sync deps/labels/comments
/// 10. Refresh blocked cache
/// 11. Update metadata
///
/// # Errors
///
/// Returns an error if:
/// - Conflict markers are detected
/// - File cannot be read
/// - Prefix validation fails
/// - Database operations fail
#[allow(clippy::too_many_lines)]
pub fn import_from_jsonl(
    storage: &mut SqliteStorage,
    input_path: &Path,
    config: &ImportConfig,
    expected_prefix: Option<&str>,
) -> Result<ImportResult> {
    use crate::util::content_hash;

    // Step 1: Conflict marker scan
    ensure_no_conflict_markers(input_path)?;

    // Step 2: Parse JSONL with 2MB buffer
    let file = File::open(input_path)?;
    let reader = BufReader::with_capacity(2 * 1024 * 1024, file);
    let mut issues = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let issue: Issue = serde_json::from_str(&line).map_err(|e| {
            BeadsError::Config(format!("Invalid JSON at line {}: {}", line_num + 1, e))
        })?;
        issues.push(issue);
    }

    let mut result = ImportResult::default();

    // Step 3: Normalize issues
    for issue in &mut issues {
        normalize_issue(issue);
    }

    // Step 4: Prefix validation (if enabled and prefix provided)
    if !config.skip_prefix_validation {
        if let Some(prefix) = expected_prefix {
            let mut mismatches = Vec::new();
            for issue in &issues {
                // Check if ID starts with expected prefix
                if !issue.id.starts_with(prefix) {
                    // Skip tombstones with wrong prefix (silently drop)
                    if issue.status == crate::model::Status::Tombstone {
                        continue;
                    }
                    mismatches.push(issue.id.clone());
                }
            }

            if !mismatches.is_empty() && !config.rename_on_import {
                return Err(BeadsError::Config(format!(
                    "Prefix mismatch: expected '{}', found issues: {}",
                    prefix,
                    mismatches
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
    }

    // Clear export hashes before importing new data.
    storage.clear_all_export_hashes()?;

    // Track external refs to detect duplicates
    let mut seen_external_refs: HashSet<String> = HashSet::new();

    // Process issues
    for issue in &issues {
        // Skip ephemerals during import (they shouldn't be in JSONL anyway)
        if issue.ephemeral {
            result.skipped_count += 1;
            continue;
        }

        let mut effective_issue = issue.clone();

        // Handle external ref duplicates before collision detection
        if let Some(ref ext_ref) = issue.external_ref {
            if seen_external_refs.contains(ext_ref) {
                if config.clear_duplicate_external_refs {
                    effective_issue.external_ref = None;
                    effective_issue.content_hash = Some(content_hash(&effective_issue));
                } else {
                    return Err(BeadsError::Config(format!(
                        "Duplicate external_ref: {ext_ref}"
                    )));
                }
            } else {
                seen_external_refs.insert(ext_ref.clone());
            }
        }

        // Compute content hash for collision detection
        let computed_hash = content_hash(&effective_issue);

        // Detect collision
        let collision = detect_collision(&effective_issue, storage, &computed_hash)?;

        // Determine action
        let action = determine_action(&collision, &effective_issue, storage, config.force_upsert)?;

        // Process the action
        process_import_action(storage, &action, &effective_issue, &mut result)?;
    }

    // Step 10: Refresh blocked cache
    storage.rebuild_blocked_cache()?;

    // Step 11: Update metadata
    storage.set_metadata(METADATA_LAST_IMPORT_TIME, &chrono::Utc::now().to_rfc3339())?;
    let jsonl_hash = compute_jsonl_hash(input_path)?;
    storage.set_metadata(METADATA_JSONL_CONTENT_HASH, &jsonl_hash)?;

    Ok(result)
}

/// Process a single import action.
fn process_import_action(
    storage: &mut SqliteStorage,
    action: &CollisionAction,
    issue: &Issue,
    result: &mut ImportResult,
) -> Result<()> {
    match action {
        CollisionAction::Insert => {
            storage.upsert_issue_for_import(issue)?;
            sync_issue_relations(storage, issue)?;
            result.imported_count += 1;
        }
        CollisionAction::Update { existing_id } => {
            // When updating by external_ref or content_hash, the incoming issue may have
            // a different ID than the existing one. We need to update using the existing ID.
            if existing_id == &issue.id {
                storage.upsert_issue_for_import(issue)?;
                sync_issue_relations(storage, issue)?;
            } else {
                let mut updated_issue = issue.clone();
                updated_issue.id.clone_from(existing_id);
                storage.upsert_issue_for_import(&updated_issue)?;
                sync_issue_relations(storage, &updated_issue)?;
            }
            result.imported_count += 1;
        }
        CollisionAction::Skip { reason } => {
            tracing::debug!(id = %issue.id, reason = %reason, "Skipping issue");
            if reason.starts_with("Tombstone") {
                result.tombstone_skipped += 1;
            } else {
                result.skipped_count += 1;
            }
        }
    }
    Ok(())
}

/// Sync labels, dependencies, and comments for an imported issue.
fn sync_issue_relations(storage: &mut SqliteStorage, issue: &Issue) -> Result<()> {
    // Sync labels
    storage.sync_labels_for_import(&issue.id, &issue.labels)?;

    // Sync dependencies
    storage.sync_dependencies_for_import(&issue.id, &issue.dependencies)?;

    // Sync comments
    storage.sync_comments_for_import(&issue.id, &issue.comments)?;

    Ok(())
}

/// Finalize an import by computing the content hash of the imported file.
///
/// # Errors
///
/// Returns an error if the file cannot be read.
pub fn compute_jsonl_hash(path: &Path) -> Result<String> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut hasher = Sha256::new();

    for line in reader.lines() {
        let line = line?;
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }

    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Issue, IssueType, Priority, Status};
    use chrono::Utc;
    use std::io::{self, Write};
    use tempfile::TempDir;

    fn make_test_issue(id: &str, title: &str) -> Issue {
        Issue {
            id: id.to_string(),
            content_hash: None,
            title: title.to_string(),
            description: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            status: Status::Open,
            priority: Priority::MEDIUM,
            issue_type: IssueType::Task,
            assignee: None,
            owner: None,
            estimated_minutes: None,
            created_at: Utc::now(),
            created_by: None,
            updated_at: Utc::now(),
            closed_at: None,
            close_reason: None,
            closed_by_session: None,
            due_at: None,
            defer_until: None,
            external_ref: None,
            source_system: None,
            deleted_at: None,
            deleted_by: None,
            delete_reason: None,
            original_type: None,
            compaction_level: None,
            compacted_at: None,
            compacted_at_commit: None,
            original_size: None,
            sender: None,
            ephemeral: false,
            pinned: false,
            is_template: false,
            labels: vec![],
            dependencies: vec![],
            comments: vec![],
        }
    }

    fn make_issue_at(id: &str, title: &str, updated_at: chrono::DateTime<Utc>) -> Issue {
        let created_at = updated_at - chrono::Duration::seconds(60);
        Issue {
            id: id.to_string(),
            content_hash: None,
            title: title.to_string(),
            description: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            status: Status::Open,
            priority: Priority::MEDIUM,
            issue_type: IssueType::Task,
            assignee: None,
            owner: None,
            estimated_minutes: None,
            created_at,
            created_by: None,
            updated_at,
            closed_at: None,
            close_reason: None,
            closed_by_session: None,
            due_at: None,
            defer_until: None,
            external_ref: None,
            source_system: None,
            deleted_at: None,
            deleted_by: None,
            delete_reason: None,
            original_type: None,
            compaction_level: None,
            compacted_at: None,
            compacted_at_commit: None,
            original_size: None,
            sender: None,
            ephemeral: false,
            pinned: false,
            is_template: false,
            labels: vec![],
            dependencies: vec![],
            comments: vec![],
        }
    }

    fn set_content_hash(issue: &mut Issue) {
        issue.content_hash = Some(crate::util::content_hash(issue));
    }

    fn fixed_time(secs: i64) -> chrono::DateTime<Utc> {
        chrono::DateTime::from_timestamp(secs, 0).expect("timestamp")
    }

    struct LineFailWriter {
        buffer: Vec<u8>,
        current: Vec<u8>,
        fail_on: String,
        failed: bool,
    }

    impl LineFailWriter {
        fn new(fail_on: &str) -> Self {
            Self {
                buffer: Vec::new(),
                current: Vec::new(),
                fail_on: fail_on.to_string(),
                failed: false,
            }
        }

        fn into_string(self) -> String {
            String::from_utf8(self.buffer).unwrap_or_default()
        }
    }

    impl Write for LineFailWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.current.extend_from_slice(buf);
            while let Some(pos) = self.current.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.current.drain(..=pos).collect();
                let line_str = String::from_utf8_lossy(&line);
                if !self.failed && line_str.contains(&self.fail_on) {
                    self.failed = true;
                    return Err(io::Error::other("intentional failure"));
                }
                self.buffer.extend_from_slice(&line);
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_scan_conflict_markers_detects_all_kinds() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("issues.jsonl");
        let contents = "\
{\"id\":\"bd-1\",\"title\":\"ok\"}
<<<<<<< HEAD
{\"id\":\"bd-2\",\"title\":\"conflict\"}
=======
{\"id\":\"bd-2\",\"title\":\"other\"}
>>>>>>> feature-branch
";
        fs::write(&path, contents).expect("write");

        let markers = scan_conflict_markers(&path).expect("scan");
        assert_eq!(markers.len(), 3);
        assert_eq!(markers[0].marker_type, ConflictMarkerType::Start);
        assert_eq!(markers[1].marker_type, ConflictMarkerType::Separator);
        assert_eq!(markers[2].marker_type, ConflictMarkerType::End);
        assert_eq!(markers[0].branch.as_deref(), Some("HEAD"));
        assert_eq!(markers[2].branch.as_deref(), Some("feature-branch"));
    }

    #[test]
    fn test_ensure_no_conflict_markers_errors() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("issues.jsonl");
        fs::write(&path, "<<<<<<< HEAD\n").expect("write");

        let err = ensure_no_conflict_markers(&path).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("Merge conflict markers detected"));
    }

    #[test]
    fn test_export_empty_database() {
        let storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        let config = ExportConfig::default();
        let result = export_to_jsonl(&storage, &output_path, &config).unwrap();

        assert_eq!(result.exported_count, 0);
        assert!(result.exported_ids.is_empty());
        assert!(output_path.exists());
    }

    #[test]
    fn test_export_with_issues() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        // Create test issues
        let issue1 = make_test_issue("bd-001", "First issue");
        let issue2 = make_test_issue("bd-002", "Second issue");

        storage.create_issue(&issue1, "test").unwrap();
        storage.create_issue(&issue2, "test").unwrap();

        let config = ExportConfig::default();
        let result = export_to_jsonl(&storage, &output_path, &config).unwrap();

        assert_eq!(result.exported_count, 2);
        assert!(result.exported_ids.contains(&"bd-001".to_string()));
        assert!(result.exported_ids.contains(&"bd-002".to_string()));

        // Verify content
        let read_back = read_issues_from_jsonl(&output_path).unwrap();
        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back[0].id, "bd-001");
        assert_eq!(read_back[1].id, "bd-002");
    }

    #[test]
    fn test_safety_guard_empty_over_nonempty() {
        let storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        // Create existing JSONL with issues
        let issue = make_test_issue("bd-existing", "Existing issue");
        let json = serde_json::to_string(&issue).unwrap();
        fs::write(&output_path, format!("{json}\n")).unwrap();

        // Try to export empty database (should fail)
        let config = ExportConfig {
            force: false,
            ..Default::default()
        };
        let result = export_to_jsonl(&storage, &output_path, &config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("empty database"));
    }

    #[test]
    fn test_safety_guard_with_force() {
        let storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        // Create existing JSONL with issues
        let issue = make_test_issue("bd-existing", "Existing issue");
        let json = serde_json::to_string(&issue).unwrap();
        fs::write(&output_path, format!("{json}\n")).unwrap();

        // Export with force (should succeed)
        let config = ExportConfig {
            force: true,
            ..Default::default()
        };
        let result = export_to_jsonl(&storage, &output_path, &config).unwrap();

        assert_eq!(result.exported_count, 0);
    }

    #[test]
    fn test_count_issues_in_jsonl() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.jsonl");

        // Empty file
        fs::write(&path, "").unwrap();
        assert_eq!(count_issues_in_jsonl(&path).unwrap(), 0);

        // Two issues
        let issue1 = make_test_issue("bd-1", "One");
        let issue2 = make_test_issue("bd-2", "Two");
        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&issue1).unwrap(),
            serde_json::to_string(&issue2).unwrap()
        );
        fs::write(&path, content).unwrap();
        assert_eq!(count_issues_in_jsonl(&path).unwrap(), 2);
    }

    #[test]
    fn test_get_issue_ids_from_jsonl() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.jsonl");

        let issue1 = make_test_issue("bd-aaa", "One");
        let issue2 = make_test_issue("bd-bbb", "Two");
        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&issue1).unwrap(),
            serde_json::to_string(&issue2).unwrap()
        );
        fs::write(&path, content).unwrap();

        let ids = get_issue_ids_from_jsonl(&path).unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("bd-aaa"));
        assert!(ids.contains("bd-bbb"));
    }

    #[test]
    fn test_export_excludes_ephemerals() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        // Create regular and ephemeral issues
        let regular = make_test_issue("bd-regular", "Regular issue");
        let mut ephemeral = make_test_issue("bd-ephemeral", "Ephemeral issue");
        ephemeral.ephemeral = true;

        storage.create_issue(&regular, "test").unwrap();
        storage.create_issue(&ephemeral, "test").unwrap();

        let config = ExportConfig::default();
        let result = export_to_jsonl(&storage, &output_path, &config).unwrap();

        // Only regular issue should be exported
        assert_eq!(result.exported_count, 1);
        assert!(result.exported_ids.contains(&"bd-regular".to_string()));
        assert!(!result.exported_ids.contains(&"bd-ephemeral".to_string()));
    }

    #[test]
    fn test_stale_database_guard_prevents_losing_issues() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        // Create a JSONL with two issues
        let issue1 = make_test_issue("bd-001", "First");
        let issue2 = make_test_issue("bd-002", "Second");
        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&issue1).unwrap(),
            serde_json::to_string(&issue2).unwrap()
        );
        fs::write(&output_path, content).unwrap();

        // Only create one issue in DB (missing bd-002)
        storage.create_issue(&issue1, "test").unwrap();

        // Export should fail because it would lose bd-002
        let config = ExportConfig::default();
        let result = export_to_jsonl(&storage, &output_path, &config);

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("stale database") || err.contains("lose"));
    }

    #[test]
    fn test_stale_database_guard_with_force_succeeds() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        // Create a JSONL with two issues
        let issue1 = make_test_issue("bd-001", "First");
        let issue2 = make_test_issue("bd-002", "Second");
        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&issue1).unwrap(),
            serde_json::to_string(&issue2).unwrap()
        );
        fs::write(&output_path, content).unwrap();

        // Only create one issue in DB
        storage.create_issue(&issue1, "test").unwrap();

        // Export with force should succeed
        let config = ExportConfig {
            force: true,
            ..Default::default()
        };
        let result = export_to_jsonl(&storage, &output_path, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_normalize_issue_wisp_detection() {
        let mut issue = make_test_issue("bd-wisp-123", "Wisp issue");
        assert!(!issue.ephemeral);

        normalize_issue(&mut issue);

        // Issue ID containing "-wisp-" should be marked ephemeral
        assert!(issue.ephemeral);
    }

    #[test]
    fn test_normalize_issue_closed_at_repair() {
        let mut issue = make_test_issue("bd-001", "Closed issue");
        issue.status = Status::Closed;
        issue.closed_at = None;

        normalize_issue(&mut issue);

        // closed_at should be set to updated_at for closed issues
        assert!(issue.closed_at.is_some());
        assert_eq!(issue.closed_at, Some(issue.updated_at));
    }

    #[test]
    fn test_normalize_issue_clears_closed_at_for_open() {
        let mut issue = make_test_issue("bd-001", "Open issue");
        issue.status = Status::Open;
        issue.closed_at = Some(Utc::now());

        normalize_issue(&mut issue);

        // closed_at should be cleared for open issues
        assert!(issue.closed_at.is_none());
    }

    #[test]
    fn test_normalize_issue_computes_content_hash() {
        let mut issue = make_test_issue("bd-001", "Test");
        issue.content_hash = None;

        normalize_issue(&mut issue);

        assert!(issue.content_hash.is_some());
        assert!(!issue.content_hash.as_ref().unwrap().is_empty());
    }

    #[test]
    fn test_import_collision_by_id_updates_newer() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        // Create existing issue in DB with older timestamp
        let mut existing = make_test_issue("test-001", "Old title");
        existing.updated_at = Utc::now() - chrono::Duration::hours(1);
        storage.create_issue(&existing, "test").unwrap();

        // Create JSONL with same ID but newer timestamp and new title
        let mut incoming = make_test_issue("test-001", "New title");
        incoming.updated_at = Utc::now();
        let json = serde_json::to_string(&incoming).unwrap();
        fs::write(&path, format!("{json}\n")).unwrap();

        // Import should update since incoming is newer
        let result =
            import_from_jsonl(&mut storage, &path, &ImportConfig::default(), Some("test-"))
                .unwrap();
        assert_eq!(result.imported_count, 1);

        let updated = storage.get_issue("test-001").unwrap().unwrap();
        assert_eq!(updated.title, "New title");
    }

    #[test]
    fn test_import_collision_by_id_skips_older() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        // Create existing issue in DB with newer timestamp
        let mut existing = make_test_issue("test-001", "Newer title");
        existing.updated_at = Utc::now();
        storage.create_issue(&existing, "test").unwrap();

        // Create JSONL with same ID but older timestamp
        let mut incoming = make_test_issue("test-001", "Older title");
        incoming.updated_at = Utc::now() - chrono::Duration::hours(1);
        let json = serde_json::to_string(&incoming).unwrap();
        fs::write(&path, format!("{json}\n")).unwrap();

        // Import should skip since existing is newer
        let result =
            import_from_jsonl(&mut storage, &path, &ImportConfig::default(), Some("test-"))
                .unwrap();
        assert_eq!(result.skipped_count, 1);

        let unchanged = storage.get_issue("test-001").unwrap().unwrap();
        assert_eq!(unchanged.title, "Newer title");
    }

    #[test]
    fn test_import_collision_by_external_ref_same_id() {
        // Test collision detection by external_ref when IDs also match
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        // Create existing issue with external_ref
        let mut existing = make_test_issue("test-001", "Existing");
        existing.external_ref = Some("JIRA-123".to_string());
        existing.updated_at = Utc::now() - chrono::Duration::hours(1);
        storage.create_issue(&existing, "test").unwrap();

        // Create JSONL with SAME ID and same external_ref but newer timestamp
        let mut incoming = make_test_issue("test-001", "Updated via external ref");
        incoming.external_ref = Some("JIRA-123".to_string());
        incoming.updated_at = Utc::now();
        let json = serde_json::to_string(&incoming).unwrap();
        fs::write(&path, format!("{json}\n")).unwrap();

        // Import should update since incoming is newer (matched by external_ref in phase 1)
        let config = ImportConfig::default();
        let result = import_from_jsonl(&mut storage, &path, &config, Some("test-")).unwrap();
        assert_eq!(result.imported_count, 1);

        // The existing issue should be updated
        let updated = storage.get_issue("test-001").unwrap().unwrap();
        assert_eq!(updated.title, "Updated via external ref");
    }

    #[test]
    fn test_detect_collision_by_external_ref() {
        // Test that collision detection correctly identifies external_ref matches
        let mut storage = SqliteStorage::open_memory().unwrap();

        // Create existing issue with external_ref
        let mut existing = make_test_issue("test-001", "Existing");
        existing.external_ref = Some("JIRA-123".to_string());
        storage.create_issue(&existing, "test").unwrap();

        // Incoming issue with same external_ref but different ID
        let mut incoming = make_test_issue("test-002", "Incoming");
        incoming.external_ref = Some("JIRA-123".to_string());

        let hash = crate::util::content_hash(&incoming);

        let result = detect_collision(&incoming, &storage, &hash).unwrap();

        // Should match by external_ref (phase 1)
        match result {
            CollisionResult::Match {
                existing_id,
                match_type,
                phase,
            } => {
                assert_eq!(existing_id, "test-001");
                assert_eq!(match_type, MatchType::ExternalRef);
                assert_eq!(phase, 1);
            }
            CollisionResult::NewIssue => panic!("Expected external_ref match"),
        }
    }

    #[test]
    fn test_import_tombstone_protection() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        // Create tombstone in DB
        let mut tombstone = make_test_issue("test-001", "Tombstone");
        tombstone.status = Status::Tombstone;
        tombstone.deleted_at = Some(Utc::now());
        storage.create_issue(&tombstone, "test").unwrap();

        // Create JSONL with same ID but trying to resurrect
        let mut incoming = make_test_issue("test-001", "Resurrected");
        incoming.status = Status::Open;
        incoming.updated_at = Utc::now() + chrono::Duration::hours(1);
        let json = serde_json::to_string(&incoming).unwrap();
        fs::write(&path, format!("{json}\n")).unwrap();

        // Import should skip due to tombstone protection
        let result =
            import_from_jsonl(&mut storage, &path, &ImportConfig::default(), Some("test-"))
                .unwrap();
        assert_eq!(result.tombstone_skipped, 1);

        let still_tombstone = storage.get_issue("test-001").unwrap().unwrap();
        assert_eq!(still_tombstone.status, Status::Tombstone);
    }

    #[test]
    fn test_import_new_issue_creates() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        // Create JSONL with new issue
        let new_issue = make_test_issue("test-new", "Brand new");
        let json = serde_json::to_string(&new_issue).unwrap();
        fs::write(&path, format!("{json}\n")).unwrap();

        let result =
            import_from_jsonl(&mut storage, &path, &ImportConfig::default(), Some("test-"))
                .unwrap();
        assert_eq!(result.imported_count, 1);

        let created = storage.get_issue("test-new").unwrap().unwrap();
        assert_eq!(created.title, "Brand new");
    }

    #[test]
    fn test_get_issue_ids_missing_file_returns_empty() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.jsonl");

        let ids = get_issue_ids_from_jsonl(&path).unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn test_count_issues_missing_file_returns_zero() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.jsonl");

        let count = count_issues_in_jsonl(&path).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_export_computes_content_hash() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        let issue = make_test_issue("bd-001", "Test");
        storage.create_issue(&issue, "test").unwrap();

        let config = ExportConfig::default();
        let result = export_to_jsonl(&storage, &output_path, &config).unwrap();

        // Result should include a non-empty content hash
        assert!(!result.content_hash.is_empty());
        // Hash should be hex (64 chars for SHA256)
        assert_eq!(result.content_hash.len(), 64);
    }

    #[test]
    fn test_export_deterministic_hash() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();

        let issue = make_test_issue("bd-001", "Deterministic");
        storage.create_issue(&issue, "test").unwrap();

        let config = ExportConfig::default();

        // Export twice to different files
        let path1 = temp_dir.path().join("export1.jsonl");
        let path2 = temp_dir.path().join("export2.jsonl");

        let result1 = export_to_jsonl(&storage, &path1, &config).unwrap();
        let result2 = export_to_jsonl(&storage, &path2, &config).unwrap();

        // Hashes should be identical for same content
        assert_eq!(result1.content_hash, result2.content_hash);
    }

    #[test]
    fn test_import_skips_ephemerals() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        // Create JSONL with ephemeral issue
        let mut ephemeral = make_test_issue("test-001", "Ephemeral");
        ephemeral.ephemeral = true;
        let json = serde_json::to_string(&ephemeral).unwrap();
        fs::write(&path, format!("{json}\n")).unwrap();

        let result =
            import_from_jsonl(&mut storage, &path, &ImportConfig::default(), Some("test-"))
                .unwrap();

        // Ephemeral should be skipped
        assert_eq!(result.skipped_count, 1);
        assert_eq!(result.imported_count, 0);
        assert!(storage.get_issue("test-001").unwrap().is_none());
    }

    #[test]
    fn test_import_skip_prefix_validation() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        // Create JSONL with mismatched prefix
        let issue = make_test_issue("other-001", "Other prefix");
        let json = serde_json::to_string(&issue).unwrap();
        fs::write(&path, format!("{json}\n")).unwrap();

        // Import with skip_prefix_validation should succeed
        let config = ImportConfig {
            skip_prefix_validation: true,
            ..Default::default()
        };
        let result = import_from_jsonl(&mut storage, &path, &config, Some("test-")).unwrap();
        assert_eq!(result.imported_count, 1);
    }

    #[test]
    fn test_import_handles_empty_lines() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        // Create JSONL with empty lines
        let issue = make_test_issue("test-001", "Valid");
        let json = serde_json::to_string(&issue).unwrap();
        let content = format!("\n{json}\n\n\n");
        fs::write(&path, content).unwrap();

        let result =
            import_from_jsonl(&mut storage, &path, &ImportConfig::default(), Some("test-"))
                .unwrap();
        assert_eq!(result.imported_count, 1);
    }

    #[test]
    fn test_detect_collision_external_ref_priority() {
        let mut storage = SqliteStorage::open_memory().unwrap();

        let mut ext_issue = make_issue_at("bd-ext", "External", fixed_time(100));
        ext_issue.external_ref = Some("JIRA-1".to_string());
        set_content_hash(&mut ext_issue);
        storage.upsert_issue_for_import(&ext_issue).unwrap();

        let mut hash_issue = make_issue_at("bd-hash", "Incoming", fixed_time(200));
        set_content_hash(&mut hash_issue);
        storage.upsert_issue_for_import(&hash_issue).unwrap();

        let mut incoming = make_issue_at("bd-new", "Incoming", fixed_time(300));
        incoming.external_ref = Some("JIRA-1".to_string());
        let computed_hash = crate::util::content_hash(&incoming);

        let collision = detect_collision(&incoming, &storage, &computed_hash).unwrap();
        match collision {
            CollisionResult::Match {
                existing_id,
                match_type,
                phase,
            } => {
                assert_eq!(existing_id, "bd-ext");
                assert_eq!(match_type, MatchType::ExternalRef);
                assert_eq!(phase, 1);
            }
            CollisionResult::NewIssue => panic!("expected match"),
        }
    }

    #[test]
    fn test_detect_collision_content_hash_before_id() {
        let mut storage = SqliteStorage::open_memory().unwrap();

        let mut hash_issue = make_issue_at("bd-hash", "Same Content", fixed_time(100));
        set_content_hash(&mut hash_issue);
        storage.upsert_issue_for_import(&hash_issue).unwrap();

        let mut id_issue = make_issue_at("bd-same", "Different Content", fixed_time(120));
        set_content_hash(&mut id_issue);
        storage.upsert_issue_for_import(&id_issue).unwrap();

        let incoming = make_issue_at("bd-same", "Same Content", fixed_time(200));
        let computed_hash = crate::util::content_hash(&incoming);

        let collision = detect_collision(&incoming, &storage, &computed_hash).unwrap();
        match collision {
            CollisionResult::Match {
                existing_id,
                match_type,
                phase,
            } => {
                assert_eq!(existing_id, "bd-hash");
                assert_eq!(match_type, MatchType::ContentHash);
                assert_eq!(phase, 2);
            }
            CollisionResult::NewIssue => panic!("expected match"),
        }
    }

    #[test]
    fn test_detect_collision_id_match() {
        let mut storage = SqliteStorage::open_memory().unwrap();

        let existing = make_issue_at("bd-1", "Existing", fixed_time(100));
        storage.create_issue(&existing, "test").unwrap();

        let incoming = make_issue_at("bd-1", "Incoming", fixed_time(200));
        let computed_hash = crate::util::content_hash(&incoming);

        let collision = detect_collision(&incoming, &storage, &computed_hash).unwrap();
        match collision {
            CollisionResult::Match {
                existing_id,
                match_type,
                phase,
            } => {
                assert_eq!(existing_id, "bd-1");
                assert_eq!(match_type, MatchType::Id);
                assert_eq!(phase, 3);
            }
            CollisionResult::NewIssue => panic!("expected match"),
        }
    }

    #[test]
    fn test_determine_action_tombstone_skip() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let mut tombstone = make_issue_at("bd-1", "Tombstone", fixed_time(100));
        tombstone.status = Status::Tombstone;
        storage.create_issue(&tombstone, "test").unwrap();

        let incoming = make_issue_at("bd-1", "Incoming", fixed_time(200));
        let collision = CollisionResult::Match {
            existing_id: "bd-1".to_string(),
            match_type: MatchType::Id,
            phase: 3,
        };
        let action = determine_action(&collision, &incoming, &storage, false).unwrap();
        match action {
            CollisionAction::Skip { reason } => {
                assert!(reason.contains("Tombstone protection"));
            }
            _ => panic!("expected tombstone skip"),
        }
    }

    #[test]
    fn test_determine_action_timestamp_comparison() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let existing = make_issue_at("bd-1", "Existing", fixed_time(100));
        storage.create_issue(&existing, "test").unwrap();

        let collision = CollisionResult::Match {
            existing_id: "bd-1".to_string(),
            match_type: MatchType::Id,
            phase: 3,
        };

        let newer = make_issue_at("bd-1", "Incoming", fixed_time(200));
        let action = determine_action(&collision, &newer, &storage, false).unwrap();
        match action {
            CollisionAction::Update { .. } => {}
            _ => panic!("expected update action"),
        }

        let equal = make_issue_at("bd-1", "Incoming", fixed_time(100));
        let action = determine_action(&collision, &equal, &storage, false).unwrap();
        match action {
            CollisionAction::Skip { reason } => assert!(reason.contains("Equal timestamps")),
            _ => panic!("expected equal timestamp skip"),
        }

        let older = make_issue_at("bd-1", "Incoming", fixed_time(50));
        let action = determine_action(&collision, &older, &storage, false).unwrap();
        match action {
            CollisionAction::Skip { reason } => assert!(reason.contains("Existing is newer")),
            _ => panic!("expected older timestamp skip"),
        }
    }

    #[test]
    fn test_import_prefix_mismatch_error() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        let issue = make_issue_at("xx-1", "Bad prefix", fixed_time(100));
        let json = serde_json::to_string(&issue).unwrap();
        fs::write(&path, format!("{json}\n")).unwrap();

        let config = ImportConfig::default();
        let err = import_from_jsonl(&mut storage, &path, &config, Some("bd")).unwrap_err();
        assert!(err.to_string().contains("Prefix mismatch"));
    }

    #[test]
    fn test_import_duplicate_external_ref_errors() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        let mut issue1 = make_issue_at("bd-1", "Issue 1", fixed_time(100));
        issue1.external_ref = Some("JIRA-1".to_string());
        let mut issue2 = make_issue_at("bd-2", "Issue 2", fixed_time(120));
        issue2.external_ref = Some("JIRA-1".to_string());

        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&issue1).unwrap(),
            serde_json::to_string(&issue2).unwrap()
        );
        fs::write(&path, content).unwrap();

        let config = ImportConfig::default();
        let err = import_from_jsonl(&mut storage, &path, &config, None).unwrap_err();
        assert!(err.to_string().contains("Duplicate external_ref"));
    }

    #[test]
    fn test_import_duplicate_external_ref_clears_and_inserts() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("issues.jsonl");

        let mut issue1 = make_issue_at("bd-1", "Issue 1", fixed_time(100));
        issue1.external_ref = Some("JIRA-1".to_string());
        let mut issue2 = make_issue_at("bd-2", "Issue 2", fixed_time(120));
        issue2.external_ref = Some("JIRA-1".to_string());

        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&issue1).unwrap(),
            serde_json::to_string(&issue2).unwrap()
        );
        fs::write(&path, content).unwrap();

        let config = ImportConfig {
            clear_duplicate_external_refs: true,
            ..Default::default()
        };
        let result = import_from_jsonl(&mut storage, &path, &config, None).unwrap();

        assert_eq!(result.imported_count, 2);
        let first = storage.get_issue("bd-1").unwrap().unwrap();
        let second = storage.get_issue("bd-2").unwrap().unwrap();
        assert_eq!(first.external_ref.as_deref(), Some("JIRA-1"));
        assert!(second.external_ref.is_none());
    }

    #[test]
    fn test_export_deterministic_order() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        let issue_a = make_test_issue("bd-z", "Zed");
        let issue_b = make_test_issue("bd-a", "Aye");
        let issue_c = make_test_issue("bd-m", "Em");

        storage.create_issue(&issue_a, "test").unwrap();
        storage.create_issue(&issue_b, "test").unwrap();
        storage.create_issue(&issue_c, "test").unwrap();

        let config = ExportConfig::default();
        export_to_jsonl(&storage, &output_path, &config).unwrap();

        let ids = read_issues_from_jsonl(&output_path)
            .unwrap()
            .into_iter()
            .map(|issue| issue.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["bd-a", "bd-m", "bd-z"]);
    }

    #[test]
    fn test_finalize_export_updates_metadata_and_clears_dirty() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let temp_dir = TempDir::new().unwrap();
        let output_path = temp_dir.path().join("issues.jsonl");

        let issue = make_test_issue("bd-1", "Issue");
        storage.create_issue(&issue, "test").unwrap();
        assert_eq!(storage.get_dirty_issue_ids().unwrap().len(), 1);

        let config = ExportConfig::default();
        let result = export_to_jsonl(&storage, &output_path, &config).unwrap();
        finalize_export(&mut storage, &result, Some(&result.issue_hashes)).unwrap();

        assert!(storage.get_dirty_issue_ids().unwrap().is_empty());
        assert!(
            storage
                .get_metadata(METADATA_JSONL_CONTENT_HASH)
                .unwrap()
                .is_some()
        );
        assert!(
            storage
                .get_metadata(METADATA_LAST_EXPORT_TIME)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn test_export_policy_strict_fails_on_write_error() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let issue1 = make_test_issue("bd-001", "First");
        let issue2 = make_test_issue("bd-002", "Second");
        storage.create_issue(&issue1, "test").unwrap();
        storage.create_issue(&issue2, "test").unwrap();

        let mut writer = LineFailWriter::new("bd-002");
        let result = export_to_writer_with_policy(&storage, &mut writer, ExportErrorPolicy::Strict);
        assert!(result.is_err());
    }

    #[test]
    fn test_export_policy_best_effort_skips_write_error() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let issue1 = make_test_issue("bd-001", "First");
        let issue2 = make_test_issue("bd-002", "Second");
        storage.create_issue(&issue1, "test").unwrap();
        storage.create_issue(&issue2, "test").unwrap();

        let mut writer = LineFailWriter::new("bd-002");
        let (result, report) =
            export_to_writer_with_policy(&storage, &mut writer, ExportErrorPolicy::BestEffort)
                .unwrap();
        assert_eq!(result.exported_count, 1);
        assert_eq!(report.errors.len(), 1);
        let output = writer.into_string();
        assert!(output.contains("bd-001"));
        assert!(!output.contains("bd-002"));
    }

    #[test]
    fn test_export_policy_partial_collects_write_error() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let issue1 = make_test_issue("bd-001", "First");
        let issue2 = make_test_issue("bd-002", "Second");
        storage.create_issue(&issue1, "test").unwrap();
        storage.create_issue(&issue2, "test").unwrap();

        let mut writer = LineFailWriter::new("bd-002");
        let (result, report) =
            export_to_writer_with_policy(&storage, &mut writer, ExportErrorPolicy::Partial)
                .unwrap();
        assert_eq!(result.exported_count, 1);
        assert_eq!(report.errors.len(), 1);
    }

    #[test]
    fn test_export_policy_required_core_fails_on_issue_error() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let issue1 = make_test_issue("bd-001", "First");
        let issue2 = make_test_issue("bd-002", "Second");
        storage.create_issue(&issue1, "test").unwrap();
        storage.create_issue(&issue2, "test").unwrap();

        let mut writer = LineFailWriter::new("bd-002");
        let result =
            export_to_writer_with_policy(&storage, &mut writer, ExportErrorPolicy::RequiredCore);
        assert!(result.is_err());
    }

    #[test]
    fn test_export_policy_required_core_allows_non_core_errors() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let issue = make_test_issue("bd-001", "First");
        storage.create_issue(&issue, "test").unwrap();
        storage.execute_test_sql("DROP TABLE labels").unwrap();

        let mut writer = Vec::new();
        let (result, report) =
            export_to_writer_with_policy(&storage, &mut writer, ExportErrorPolicy::RequiredCore)
                .unwrap();

        assert_eq!(result.exported_count, 1);
        assert!(
            report
                .errors
                .iter()
                .any(|err| err.entity_type == ExportEntityType::Label)
        );
    }
}
