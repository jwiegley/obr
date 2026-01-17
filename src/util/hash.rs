//! Content hashing for issue deduplication and sync.
//!
//! Uses SHA256 over stable ordered fields with null separators.
//! Matches classic bd behavior for export/import compatibility.

use sha2::{Digest, Sha256};

use crate::model::{Issue, IssueType, Priority, Status};

/// Trait for types that can produce a deterministic content hash.
pub trait ContentHashable {
    /// Compute the content hash for this value.
    fn content_hash(&self) -> String;
}

impl ContentHashable for Issue {
    fn content_hash(&self) -> String {
        content_hash(self)
    }
}

/// Compute SHA256 content hash for an issue.
///
/// Fields included (stable order with null separators):
/// - title, description, design, `acceptance_criteria`, notes
/// - status, priority, `issue_type`
/// - assignee, `external_ref`
/// - pinned, `is_template`
///
/// Fields excluded:
/// - id, `content_hash` (circular)
/// - labels, dependencies, comments, events (separate entities)
/// - timestamps (`created_at`, `updated_at`, `closed_at`, etc.)
/// - tombstone fields (`deleted_at`, `deleted_by`, `delete_reason`)
/// - owner, `created_by` (metadata, not content)
#[must_use]
pub fn content_hash(issue: &Issue) -> String {
    let mut hasher = Sha256::new();

    // Helper to add field with null separator
    // Sanitizes input by replacing null bytes to prevent collision attacks
    let mut add_field = |value: &str| {
        if value.contains('\0') {
            // Replace null with space to preserve length approx and prevent separator confusion
            hasher.update(value.replace('\0', " ").as_bytes());
        } else {
            hasher.update(value.as_bytes());
        }
        hasher.update(b"\x00");
    };

    // Title (required)
    add_field(&issue.title);

    // Optional text fields
    add_field(issue.description.as_deref().unwrap_or(""));
    add_field(issue.design.as_deref().unwrap_or(""));
    add_field(issue.acceptance_criteria.as_deref().unwrap_or(""));
    add_field(issue.notes.as_deref().unwrap_or(""));

    // Status as string
    add_field(issue.status.as_str());

    // Priority as string (P0-P4)
    add_field(&format!("P{}", issue.priority.0));

    // Issue type as string
    add_field(issue.issue_type.as_str());

    // Optional fields
    add_field(issue.assignee.as_deref().unwrap_or(""));
    add_field(issue.external_ref.as_deref().unwrap_or(""));

    // Boolean flags as "true"/"false"
    add_field(if issue.pinned { "true" } else { "false" });
    add_field(if issue.is_template { "true" } else { "false" });

    // Return lowercase hex
    format!("{:x}", hasher.finalize())
}

/// Create a content hash from raw components (for import/validation).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn content_hash_from_parts(
    title: &str,
    description: Option<&str>,
    design: Option<&str>,
    acceptance_criteria: Option<&str>,
    notes: Option<&str>,
    status: &Status,
    priority: &Priority,
    issue_type: &IssueType,
    assignee: Option<&str>,
    external_ref: Option<&str>,
    pinned: bool,
    is_template: bool,
) -> String {
    let mut hasher = Sha256::new();

    let mut add_field = |value: &str| {
        if value.contains('\0') {
            hasher.update(value.replace('\0', " ").as_bytes());
        } else {
            hasher.update(value.as_bytes());
        }
        hasher.update(b"\x00");
    };

    add_field(title);
    add_field(description.unwrap_or(""));
    add_field(design.unwrap_or(""));
    add_field(acceptance_criteria.unwrap_or(""));
    add_field(notes.unwrap_or(""));
    add_field(status.as_str());
    add_field(&format!("P{}", priority.0));
    add_field(issue_type.as_str());
    add_field(assignee.unwrap_or(""));
    add_field(external_ref.unwrap_or(""));
    add_field(if pinned { "true" } else { "false" });
    add_field(if is_template { "true" } else { "false" });

    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_test_issue() -> Issue {
        Issue {
            id: "bd-test123".to_string(),
            content_hash: None,
            title: "Test Issue".to_string(),
            description: Some("A test description".to_string()),
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

    #[test]
    fn test_content_hash_deterministic() {
        let issue = make_test_issue();
        let hash1 = content_hash(&issue);
        let hash2 = content_hash(&issue);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_is_hex() {
        let issue = make_test_issue();
        let hash = content_hash(&issue);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash.len(), 64); // SHA256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn test_content_hash_changes_with_title() {
        let mut issue = make_test_issue();
        let hash1 = content_hash(&issue);

        issue.title = "Different Title".to_string();
        let hash2 = content_hash(&issue);

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_ignores_timestamps() {
        let mut issue = make_test_issue();
        let hash1 = content_hash(&issue);

        issue.updated_at = Utc::now();
        let hash2 = content_hash(&issue);

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_includes_pinned() {
        let mut issue = make_test_issue();
        let hash1 = content_hash(&issue);

        issue.pinned = true;
        let hash2 = content_hash(&issue);

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_from_parts() {
        let issue = make_test_issue();
        let direct = content_hash(&issue);
        let from_parts = content_hash_from_parts(
            &issue.title,
            issue.description.as_deref(),
            issue.design.as_deref(),
            issue.acceptance_criteria.as_deref(),
            issue.notes.as_deref(),
            &issue.status,
            &issue.priority,
            &issue.issue_type,
            issue.assignee.as_deref(),
            issue.external_ref.as_deref(),
            issue.pinned,
            issue.is_template,
        );
        assert_eq!(direct, from_parts);
    }
}
