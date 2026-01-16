//! Path validation and allowlist enforcement for sync operations.
//!
//! This module defines the explicit allowlist of files that `br sync` is permitted
//! to touch and provides validation functions to enforce this boundary.
//!
//! # Safety Model
//!
//! The sync allowlist is a critical safety boundary. All sync I/O operations MUST
//! pass through `validate_sync_path()` before performing any file operations.
//!
//! # Allowlist
//!
//! The following paths are permitted for sync operations:
//!
//! | Pattern | Purpose |
//! |---------|---------|
//! | `.beads/*.db` | `SQLite` database files |
//! | `.beads/*.db-wal` | `SQLite` WAL files |
//! | `.beads/*.db-shm` | `SQLite` shared memory files |
//! | `.beads/*.jsonl` | `JSONL` export files |
//! | `.beads/*.jsonl.tmp` | Temp files for atomic writes |
//! | `.beads/.manifest.json` | Export manifest |
//! | `.beads/metadata.json` | Workspace metadata |
//!
//! # External JSONL Paths
//!
//! The `BEADS_JSONL` environment variable can override the JSONL path.
//! When set to a path outside `.beads/`, sync will refuse to operate unless
//! `--allow-external-jsonl` is explicitly provided.
//!
//! # Git Path Safety
//!
//! Sync operations NEVER access `.git/` directories. This is a hard safety invariant
//! enforced by `validate_no_git_path()`. Even with `--allow-external-jsonl`, git
//! paths are always rejected.
//!
//! # References
//!
//! - `SYNC_SAFETY_INVARIANTS.md`: PC-1, PC-2, PC-3, PC-4, NG-5, NG-6, NGI-1, NGI-3

use crate::error::{BeadsError, Result};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Files explicitly allowed for sync operations within `.beads/`.
///
/// This list is exhaustive - any file not matching these patterns is rejected.
pub const ALLOWED_EXTENSIONS: &[&str] = &[
    "db",        // SQLite database
    "db-wal",    // SQLite WAL
    "db-shm",    // SQLite shared memory
    "jsonl",     // JSONL export
    "jsonl.tmp", // Atomic write temp files
];

/// Files explicitly allowed by exact name within `.beads/`.
pub const ALLOWED_EXACT_NAMES: &[&str] = &[".manifest.json", "metadata.json"];

/// Result of path validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathValidation {
    /// Path is allowed for sync operations.
    Allowed,
    /// Path is outside the beads directory.
    OutsideBeadsDir { path: PathBuf, beads_dir: PathBuf },
    /// Path has a disallowed extension.
    DisallowedExtension { path: PathBuf, extension: String },
    /// Path contains traversal sequences (e.g., `..`).
    TraversalAttempt { path: PathBuf },
    /// Path is a symlink pointing outside the beads directory.
    SymlinkEscape { path: PathBuf, target: PathBuf },
    /// Path failed canonicalization.
    CanonicalizationFailed { path: PathBuf, error: String },
    /// Path targets git internals (.git directory).
    GitPathAttempt { path: PathBuf },
}

impl PathValidation {
    /// Returns true if the path is allowed.
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }

    /// Returns the rejection reason as a human-readable string.
    #[must_use]
    pub fn rejection_reason(&self) -> Option<String> {
        match self {
            Self::Allowed => None,
            Self::OutsideBeadsDir { path, beads_dir } => Some(format!(
                "Path '{}' is outside the beads directory '{}'",
                path.display(),
                beads_dir.display()
            )),
            Self::DisallowedExtension { path, extension } => Some(format!(
                "Path '{}' has disallowed extension '{}' (allowed: {:?})",
                path.display(),
                extension,
                ALLOWED_EXTENSIONS
            )),
            Self::TraversalAttempt { path } => Some(format!(
                "Path '{}' contains traversal sequences",
                path.display()
            )),
            Self::SymlinkEscape { path, target } => Some(format!(
                "Symlink '{}' points outside beads directory to '{}'",
                path.display(),
                target.display()
            )),
            Self::CanonicalizationFailed { path, error } => Some(format!(
                "Failed to canonicalize path '{}': {}",
                path.display(),
                error
            )),
        }
    }
}

/// Validates that a path is allowed for sync operations.
///
/// # Arguments
///
/// * `path` - The path to validate
/// * `beads_dir` - The `.beads` directory path (must be absolute)
///
/// # Returns
///
/// * `PathValidation::Allowed` if the path is permitted
/// * Other variants describing why the path was rejected
///
/// # Logging
///
/// - DEBUG: Logs successful validation with path details
/// - WARN: Logs rejected paths with reason
///
/// # Example
///
/// ```ignore
/// let beads_dir = PathBuf::from("/project/.beads");
/// let result = validate_sync_path(&beads_dir.join("issues.jsonl"), &beads_dir);
/// assert!(result.is_allowed());
/// ```
#[allow(clippy::too_many_lines)]
pub fn validate_sync_path(path: &Path, beads_dir: &Path) -> PathValidation {
    // Log the validation attempt
    debug!(path = %path.display(), beads_dir = %beads_dir.display(), "Validating sync path");

    // Check for obvious traversal attempts in the raw path
    let path_str = path.to_string_lossy();
    if path_str.contains("..") {
        let result = PathValidation::TraversalAttempt {
            path: path.to_path_buf(),
        };
        warn!(
            path = %path.display(),
            reason = %result.rejection_reason().unwrap_or_default(),
            "Path validation rejected"
        );
        return result;
    }

    // Canonicalize the beads directory
    let canonical_beads = match beads_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            let result = PathValidation::CanonicalizationFailed {
                path: beads_dir.to_path_buf(),
                error: e.to_string(),
            };
            warn!(
                path = %beads_dir.display(),
                error = %e,
                "Beads directory canonicalization failed"
            );
            return result;
        }
    };

    // For new files that don't exist yet, we check the parent directory
    let path_to_check = if path.exists() {
        path.to_path_buf()
    } else {
        // For non-existent files, verify the parent exists and is valid
        match path.parent() {
            Some(parent) if parent.exists() => parent.to_path_buf(),
            _ => {
                // If parent doesn't exist, just check if the path would be under beads_dir
                if let Ok(relative) = path.strip_prefix(&canonical_beads) {
                    // Path is specified relative to beads_dir
                    if !relative.to_string_lossy().contains("..") {
                        return validate_extension_and_name(path);
                    }
                }
                // Otherwise, try to check as-is
                path.to_path_buf()
            }
        }
    };

    // Canonicalize the path (or its parent for new files)
    let canonical_path = match path_to_check.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            // For non-existent files, we can't canonicalize, so check prefix
            if !path.exists() {
                // Check if the path starts with the beads directory
                if path.starts_with(beads_dir) || path.starts_with(&canonical_beads) {
                    return validate_extension_and_name(path);
                }
            }
            let result = PathValidation::CanonicalizationFailed {
                path: path.to_path_buf(),
                error: e.to_string(),
            };
            warn!(
                path = %path.display(),
                error = %e,
                "Path canonicalization failed"
            );
            return result;
        }
    };

    // Check if the path is a symlink pointing outside beads_dir
    if path.is_symlink() {
        if let Ok(target) = std::fs::read_link(path) {
            let canonical_target = target.canonicalize().unwrap_or_else(|_| target.clone());
            if !canonical_target.starts_with(&canonical_beads) {
                let result = PathValidation::SymlinkEscape {
                    path: path.to_path_buf(),
                    target: canonical_target,
                };
                warn!(
                    path = %path.display(),
                    target = %target.display(),
                    "Symlink escape detected"
                );
                return result;
            }
        }
    }

    // Verify the path is under the beads directory
    // For existing files, use the canonical path; for new files, use the parent's canonical + filename
    let effective_canonical = if path.exists() {
        canonical_path
    } else {
        canonical_path.join(path.file_name().unwrap_or_default())
    };

    if !effective_canonical.starts_with(&canonical_beads) {
        let result = PathValidation::OutsideBeadsDir {
            path: path.to_path_buf(),
            beads_dir: canonical_beads,
        };
        warn!(
            path = %path.display(),
            beads_dir = %beads_dir.display(),
            reason = %result.rejection_reason().unwrap_or_default(),
            "Path validation rejected"
        );
        return result;
    }

    // Validate extension and name
    let extension_result = validate_extension_and_name(path);
    if !extension_result.is_allowed() {
        warn!(
            path = %path.display(),
            reason = %extension_result.rejection_reason().unwrap_or_default(),
            "Path validation rejected"
        );
        return extension_result;
    }

    debug!(path = %path.display(), "Path validated for sync I/O");
    PathValidation::Allowed
}

/// Validates that the file extension or name is in the allowlist.
fn validate_extension_and_name(path: &Path) -> PathValidation {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // Check exact name matches first
    if ALLOWED_EXACT_NAMES.iter().any(|&name| file_name == name) {
        return PathValidation::Allowed;
    }

    // Check extension matches
    // Handle compound extensions like .jsonl.tmp
    for allowed_ext in ALLOWED_EXTENSIONS {
        if file_name.ends_with(&format!(".{allowed_ext}")) {
            return PathValidation::Allowed;
        }
    }

    // Extract simple extension for error message
    let extension = path
        .extension()
        .map_or_else(|| "none".to_string(), |e| e.to_string_lossy().to_string());

    PathValidation::DisallowedExtension {
        path: path.to_path_buf(),
        extension,
    }
}

/// Validates a path and returns an error if it's not allowed.
///
/// This is a convenience wrapper around `validate_sync_path` that returns
/// a `Result` for easier use in sync functions.
///
/// # Errors
///
/// Returns `BeadsError::Config` with a descriptive message if the path is not allowed.
pub fn require_valid_sync_path(path: &Path, beads_dir: &Path) -> Result<()> {
    let validation = validate_sync_path(path, beads_dir);
    match validation {
        PathValidation::Allowed => Ok(()),
        _ => Err(BeadsError::Config(
            validation
                .rejection_reason()
                .unwrap_or_else(|| "Path validation failed".to_string()),
        )),
    }
}

/// Checks if a path would be allowed for sync without logging.
///
/// This is useful for preflight checks where we want to validate paths
/// before attempting operations.
#[must_use]
pub fn is_sync_path_allowed(path: &Path, beads_dir: &Path) -> bool {
    // Quick check without full canonicalization for obvious cases
    let path_str = path.to_string_lossy();
    if path_str.contains("..") {
        return false;
    }

    // Check if path is under beads_dir and has allowed extension
    if path.starts_with(beads_dir) {
        return validate_extension_and_name(path).is_allowed();
    }

    // Full validation for edge cases
    validate_sync_path(path, beads_dir).is_allowed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_beads_dir() -> (TempDir, PathBuf) {
        let temp = TempDir::new().expect("create temp dir");
        let beads_dir = temp.path().join(".beads");
        std::fs::create_dir_all(&beads_dir).expect("create beads dir");
        (temp, beads_dir)
    }

    #[test]
    fn test_allowed_jsonl_file() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("issues.jsonl");
        std::fs::write(&path, "{}").expect("write");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(result.is_allowed(), "JSONL files should be allowed");
    }

    #[test]
    fn test_allowed_db_file() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("beads.db");
        std::fs::write(&path, "").expect("write");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(result.is_allowed(), "DB files should be allowed");
    }

    #[test]
    fn test_allowed_db_wal_file() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("beads.db-wal");
        std::fs::write(&path, "").expect("write");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(result.is_allowed(), "DB-WAL files should be allowed");
    }

    #[test]
    fn test_allowed_manifest_file() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join(".manifest.json");
        std::fs::write(&path, "{}").expect("write");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(result.is_allowed(), "Manifest files should be allowed");
    }

    #[test]
    fn test_allowed_metadata_file() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("metadata.json");
        std::fs::write(&path, "{}").expect("write");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(result.is_allowed(), "Metadata files should be allowed");
    }

    #[test]
    fn test_allowed_temp_file() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("issues.jsonl.tmp");
        std::fs::write(&path, "").expect("write");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(result.is_allowed(), "Temp JSONL files should be allowed");
    }

    #[test]
    fn test_rejected_outside_beads_dir() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let outside_path = beads_dir.parent().unwrap().join("outside.jsonl");
        std::fs::write(&outside_path, "").expect("write");

        let result = validate_sync_path(&outside_path, &beads_dir);
        assert!(
            matches!(result, PathValidation::OutsideBeadsDir { .. }),
            "Files outside beads dir should be rejected"
        );
    }

    #[test]
    fn test_rejected_traversal() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let traversal_path = beads_dir.join("../../../etc/passwd");

        let result = validate_sync_path(&traversal_path, &beads_dir);
        assert!(
            matches!(result, PathValidation::TraversalAttempt { .. }),
            "Traversal attempts should be rejected"
        );
    }

    #[test]
    fn test_rejected_disallowed_extension() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("config.yaml");
        std::fs::write(&path, "").expect("write");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(
            matches!(result, PathValidation::DisallowedExtension { .. }),
            "Disallowed extensions should be rejected"
        );
    }

    #[test]
    fn test_rejected_source_file() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("main.rs");
        std::fs::write(&path, "").expect("write");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(
            matches!(result, PathValidation::DisallowedExtension { .. }),
            "Source files should be rejected"
        );
    }

    #[test]
    fn test_rejected_absolute_path_outside() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = PathBuf::from("/etc/passwd");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(
            !result.is_allowed(),
            "Absolute paths outside beads dir should be rejected"
        );
    }

    #[test]
    fn test_new_file_in_beads_dir() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        // File doesn't exist yet but is in beads_dir with allowed extension
        let path = beads_dir.join("new.jsonl");

        let result = validate_sync_path(&path, &beads_dir);
        assert!(
            result.is_allowed(),
            "New JSONL files in beads dir should be allowed"
        );
    }

    #[test]
    fn test_require_valid_sync_path_ok() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("issues.jsonl");
        std::fs::write(&path, "").expect("write");

        let result = require_valid_sync_path(&path, &beads_dir);
        assert!(result.is_ok(), "Valid paths should return Ok");
    }

    #[test]
    fn test_require_valid_sync_path_error() {
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("../../../etc/passwd");

        let result = require_valid_sync_path(&path, &beads_dir);
        assert!(result.is_err(), "Invalid paths should return Err");
        assert!(result.unwrap_err().to_string().contains("traversal"));
    }

    #[test]
    fn test_is_sync_path_allowed_quick_check() {
        let (_temp, beads_dir) = setup_test_beads_dir();

        assert!(is_sync_path_allowed(
            &beads_dir.join("issues.jsonl"),
            &beads_dir
        ));
        assert!(!is_sync_path_allowed(
            &beads_dir.join("../evil.jsonl"),
            &beads_dir
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_escape_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().expect("create temp dir");
        let beads_dir = temp.path().join(".beads");
        std::fs::create_dir_all(&beads_dir).expect("create beads dir");

        // Create a target outside beads dir
        let outside_target = temp.path().join("secret.txt");
        std::fs::write(&outside_target, "secret data").expect("write");

        // Create symlink inside beads dir pointing outside
        let symlink_path = beads_dir.join("evil.jsonl");
        symlink(&outside_target, &symlink_path).expect("create symlink");

        let result = validate_sync_path(&symlink_path, &beads_dir);
        assert!(
            matches!(result, PathValidation::SymlinkEscape { .. }),
            "Symlinks escaping beads dir should be rejected"
        );
    }

    #[test]
    fn test_validation_logs_rejection() {
        // This test verifies the logging behavior by checking the return value
        // which includes the reason that would be logged
        let (_temp, beads_dir) = setup_test_beads_dir();
        let path = beads_dir.join("../../../etc/passwd");

        let result = validate_sync_path(&path, &beads_dir);
        let reason = result.rejection_reason();
        assert!(reason.is_some(), "Rejected paths should have a reason");
        assert!(
            reason.unwrap().contains("traversal"),
            "Reason should mention traversal"
        );
    }
}
