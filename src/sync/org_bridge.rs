//! Bridge module for converting between beads `Issue` format and Org-mode format.
//!
//! This module provides two main functions:
//! - `issues_to_org_text()`: Converts Issue structs to Org-mode text
//! - `org_text_to_issues()`: Parses Org-mode text into Issue structs
//!
//! The Org format is more human-readable than JSONL while preserving all issue data.

use crate::error::{BeadsError, Result};
use crate::model::{Issue, IssueType, Priority, Status};
use chrono::{DateTime, Utc};
use org2jsonl::model::{Element, EntryContent, Heading, InlineContent};
use std::str::FromStr;

/// Convert a collection of issues to Org-mode text format.
///
/// The output starts with a file header specifying TODO keywords, followed by
/// one level-1 heading per issue. Each heading includes:
/// - Status keyword (TODO/DONE/etc)
/// - Priority tag ([#A], [#B], etc)
/// - Title
/// - Tags (labels)
/// - Properties drawer (metadata)
/// - Body paragraphs (description)
/// - Child headings for design, acceptance criteria, notes
/// - JSON src blocks for dependencies and comments
#[must_use]
pub fn issues_to_org_text(issues: &[Issue]) -> String {
    let mut output = String::new();

    // File header with TODO sequence
    output.push_str("#+TITLE: Beads Issues\n");
    output.push_str("#+SEQ_TODO: OPEN IN_PROGRESS BLOCKED DEFERRED PINNED | CLOSED TOMBSTONE\n");
    output.push('\n');

    for issue in issues {
        emit_issue_heading(&mut output, issue);
    }

    output
}

/// Emit a single issue as an Org heading.
#[allow(clippy::too_many_lines)]
fn emit_issue_heading(output: &mut String, issue: &Issue) {
    // Build heading line: * STATUS [#PRIORITY] Title :tag1:tag2:
    output.push_str("* ");

    // Status keyword
    let keyword = status_to_keyword(&issue.status);
    output.push_str(&keyword);
    output.push(' ');

    // Priority
    let priority = priority_to_org(issue.priority);
    output.push_str(&priority);
    output.push(' ');

    // Title (sanitize to prevent newlines breaking the heading line)
    output.push_str(&sanitize_property_value(&issue.title));

    // Tags (labels)
    if !issue.labels.is_empty() {
        output.push_str("    :");
        let mut sorted_labels = issue.labels.clone();
        sorted_labels.sort();
        for (i, label) in sorted_labels.iter().enumerate() {
            if i > 0 {
                output.push(':');
            }
            output.push_str(label);
        }
        output.push(':');
    }

    output.push('\n');

    // Properties drawer
    output.push_str(":PROPERTIES:\n");
    output.push_str(":BEADS_SCHEMA_VERSION: 1\n");
    output.push_str(&format!(":ID:       {}\n", sanitize_property_value(&issue.id)));
    output.push_str(&format!(":ISSUE_TYPE: {}\n", issue.issue_type.as_str()));
    output.push_str(&format!(":CREATED_AT: {}\n", issue.created_at.to_rfc3339()));
    output.push_str(&format!(":UPDATED_AT: {}\n", issue.updated_at.to_rfc3339()));

    // Optional properties (all string values sanitized to prevent drawer injection)
    if let Some(ref assignee) = issue.assignee {
        output.push_str(&format!(":ASSIGNEE: {}\n", sanitize_property_value(assignee)));
    }
    if let Some(ref owner) = issue.owner {
        output.push_str(&format!(":OWNER: {}\n", sanitize_property_value(owner)));
    }
    if let Some(minutes) = issue.estimated_minutes {
        output.push_str(&format!(":ESTIMATED_MINUTES: {minutes}\n"));
    }
    if let Some(ref created_by) = issue.created_by {
        output.push_str(&format!(":CREATED_BY: {}\n", sanitize_property_value(created_by)));
    }
    if let Some(closed_at) = issue.closed_at {
        output.push_str(&format!(":CLOSED_AT: {}\n", closed_at.to_rfc3339()));
    }
    if let Some(ref close_reason) = issue.close_reason {
        output.push_str(&format!(":CLOSE_REASON: {}\n", sanitize_property_value(close_reason)));
    }
    if let Some(ref closed_by_session) = issue.closed_by_session {
        output.push_str(&format!(":CLOSED_BY_SESSION: {}\n", sanitize_property_value(closed_by_session)));
    }
    if let Some(due_at) = issue.due_at {
        output.push_str(&format!(":DUE_AT: {}\n", due_at.to_rfc3339()));
    }
    if let Some(defer_until) = issue.defer_until {
        output.push_str(&format!(":DEFER_UNTIL: {}\n", defer_until.to_rfc3339()));
    }
    if let Some(ref external_ref) = issue.external_ref {
        output.push_str(&format!(":EXTERNAL_REF: {}\n", sanitize_property_value(external_ref)));
    }
    if let Some(ref source_system) = issue.source_system {
        output.push_str(&format!(":SOURCE_SYSTEM: {}\n", sanitize_property_value(source_system)));
    }
    if let Some(ref source_repo) = issue.source_repo {
        output.push_str(&format!(":SOURCE_REPO: {}\n", sanitize_property_value(source_repo)));
    }
    if let Some(deleted_at) = issue.deleted_at {
        output.push_str(&format!(":DELETED_AT: {}\n", deleted_at.to_rfc3339()));
    }
    if let Some(ref deleted_by) = issue.deleted_by {
        output.push_str(&format!(":DELETED_BY: {}\n", sanitize_property_value(deleted_by)));
    }
    if let Some(ref delete_reason) = issue.delete_reason {
        output.push_str(&format!(":DELETE_REASON: {}\n", sanitize_property_value(delete_reason)));
    }
    if let Some(ref original_type) = issue.original_type {
        output.push_str(&format!(":ORIGINAL_TYPE: {}\n", sanitize_property_value(original_type)));
    }

    // Compaction fields (only emit when > 0)
    if let Some(level) = issue.compaction_level {
        if level > 0 {
            output.push_str(&format!(":COMPACTION_LEVEL: {level}\n"));
        }
    }
    if let Some(compacted_at) = issue.compacted_at {
        output.push_str(&format!(":COMPACTED_AT: {}\n", compacted_at.to_rfc3339()));
    }
    if let Some(ref commit) = issue.compacted_at_commit {
        output.push_str(&format!(":COMPACTED_AT_COMMIT: {}\n", sanitize_property_value(commit)));
    }
    if let Some(size) = issue.original_size {
        output.push_str(&format!(":ORIGINAL_SIZE: {size}\n"));
    }

    // Messaging
    if let Some(ref sender) = issue.sender {
        output.push_str(&format!(":SENDER: {}\n", sanitize_property_value(sender)));
    }
    if issue.ephemeral {
        output.push_str(":EPHEMERAL: true\n");
    }

    // Context
    if issue.pinned {
        output.push_str(":PINNED: true\n");
    }
    if issue.is_template {
        output.push_str(":IS_TEMPLATE: true\n");
    }

    output.push_str(":END:\n");

    // Description (body paragraphs)
    if let Some(ref desc) = issue.description {
        output.push('\n');
        let sanitized = sanitize_org_text(desc);
        output.push_str(&sanitized);
        output.push('\n');
    }

    // Design section
    if let Some(ref design) = issue.design {
        if !design.is_empty() {
            output.push('\n');
            output.push_str("** Design\n");
            let sanitized = sanitize_org_text(design);
            output.push_str(&sanitized);
            output.push('\n');
        }
    }

    // Acceptance Criteria section
    if let Some(ref ac) = issue.acceptance_criteria {
        if !ac.is_empty() {
            output.push('\n');
            output.push_str("** Acceptance Criteria\n");
            let sanitized = sanitize_org_text(ac);
            output.push_str(&sanitized);
            output.push('\n');
        }
    }

    // Notes section
    if let Some(ref notes) = issue.notes {
        if !notes.is_empty() {
            output.push('\n');
            output.push_str("** Notes\n");
            let sanitized = sanitize_org_text(notes);
            output.push_str(&sanitized);
            output.push('\n');
        }
    }

    // Dependencies section
    if !issue.dependencies.is_empty() {
        output.push('\n');
        output.push_str("** Dependencies\n");
        output.push_str("#+begin_src json\n");
        let json = serde_json::to_string_pretty(&issue.dependencies)
            .unwrap_or_else(|_| "[]".to_string());
        output.push_str(&json);
        output.push('\n');
        output.push_str("#+end_src\n");
    }

    // Comments section
    if !issue.comments.is_empty() {
        output.push('\n');
        output.push_str("** Comments\n");
        output.push_str("#+begin_src json\n");
        let json = serde_json::to_string_pretty(&issue.comments)
            .unwrap_or_else(|_| "[]".to_string());
        output.push_str(&json);
        output.push('\n');
        output.push_str("#+end_src\n");
    }

    // Blank line after issue
    output.push('\n');
}

/// Sanitize a property value for safe inclusion in an Org property drawer.
/// Strips newlines and `:END:` tokens to prevent drawer injection.
fn sanitize_property_value(value: &str) -> String {
    value
        .replace(['\n', '\r'], " ")
        .replace(":END:", ":END ")
}

/// Sanitize text for Org-mode (escape lines starting with `*`).
fn sanitize_org_text(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.starts_with('*') {
                format!(",{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Convert Issue status to Org keyword.
fn status_to_keyword(status: &Status) -> String {
    match status {
        Status::Open => "OPEN".to_string(),
        Status::InProgress => "IN_PROGRESS".to_string(),
        Status::Blocked => "BLOCKED".to_string(),
        Status::Deferred => "DEFERRED".to_string(),
        Status::Closed => "CLOSED".to_string(),
        Status::Tombstone => "TOMBSTONE".to_string(),
        Status::Pinned => "PINNED".to_string(),
        Status::Custom(s) => s.to_uppercase(),
    }
}

/// Convert Org keyword to Issue status.
fn keyword_to_status(keyword: &str) -> Result<Status> {
    match keyword.to_uppercase().as_str() {
        "OPEN" => Ok(Status::Open),
        "IN_PROGRESS" | "INPROGRESS" => Ok(Status::InProgress),
        "BLOCKED" => Ok(Status::Blocked),
        "DEFERRED" => Ok(Status::Deferred),
        "CLOSED" => Ok(Status::Closed),
        "TOMBSTONE" => Ok(Status::Tombstone),
        "PINNED" => Ok(Status::Pinned),
        _ => Status::from_str(keyword),
    }
}

/// Convert Issue priority to Org priority.
fn priority_to_org(priority: Priority) -> String {
    match priority.0 {
        0 => "[#A]".to_string(),
        1 => "[#B]".to_string(),
        3 => "[#D]".to_string(),
        4 => "[#E]".to_string(),
        _ => "[#C]".to_string(), // 2 = medium, default to medium for invalid values
    }
}

/// Convert Org priority to Issue priority.
fn org_to_priority(org_priority: Option<&str>) -> Priority {
    match org_priority {
        Some("A") => Priority::CRITICAL,
        Some("B") => Priority::HIGH,
        Some("D") => Priority::LOW,
        Some("E") => Priority::BACKLOG,
        _ => Priority::MEDIUM, // Default for C or unknown
    }
}

/// TODO keywords used in beads Org files (active/incomplete states).
pub const BEADS_TODO_KEYWORDS: &[&str] = &[
    "OPEN",
    "IN_PROGRESS",
    "BLOCKED",
    "DEFERRED",
    "PINNED",
];

/// DONE keywords used in beads Org files (completed states).
pub const BEADS_DONE_KEYWORDS: &[&str] = &[
    "CLOSED",
    "TOMBSTONE",
];

/// Parse Org-mode text into a collection of issues.
///
/// Uses the org2jsonl parser to extract headings, then converts each
/// level-1 heading into an Issue struct.
///
/// # Errors
///
/// Returns an error if the Org text cannot be parsed or if any heading
/// is missing required properties (e.g., ID).
pub fn org_text_to_issues(org_text: &str) -> Result<Vec<Issue>> {
    let entries = org2jsonl::org_to_json::org_to_entries_with_keywords(
        org_text,
        BEADS_TODO_KEYWORDS,
        BEADS_DONE_KEYWORDS,
    );
    let mut issues = Vec::new();

    for entry in entries {
        if let EntryContent::Heading(heading) = entry.content {
            // Only process level-1 headings (top-level issues)
            if heading.level == 1 {
                let issue = parse_heading_to_issue(&heading)?;
                issues.push(issue);
            }
        }
        // Skip Section entries (file header)
    }

    Ok(issues)
}

/// Convert an Org heading to an Issue.
fn parse_heading_to_issue(heading: &Heading) -> Result<Issue> {
    let mut issue = Issue::default();

    // Extract status from keyword
    if let Some(ref keyword) = heading.keyword {
        issue.status = keyword_to_status(keyword)?;

        // If status is Pinned, also set the pinned flag
        if issue.status == Status::Pinned {
            issue.pinned = true;
        }
    }

    // Extract priority
    issue.priority = org_to_priority(heading.priority.as_deref());

    // Extract title (join all text content)
    issue.title = extract_title_text(&heading.title);

    // Extract labels from tags
    issue.labels.clone_from(&heading.tags);
    issue.labels.sort();

    // Extract properties
    parse_properties(&heading.properties, &mut issue)?;

    // Extract description from body paragraphs
    let body_text = extract_body_text(&heading.body);
    if !body_text.is_empty() {
        issue.description = Some(unsanitize_org_text(&body_text));
    }

    // Extract child sections
    for child in &heading.children {
        if child.level == 2 {
            let child_title = extract_title_text(&child.title);
            let child_body = extract_body_text(&child.body);

            match child_title.as_str() {
                "Design" => {
                    if !child_body.is_empty() {
                        issue.design = Some(unsanitize_org_text(&child_body));
                    }
                }
                "Acceptance Criteria" => {
                    if !child_body.is_empty() {
                        issue.acceptance_criteria = Some(unsanitize_org_text(&child_body));
                    }
                }
                "Notes" => {
                    if !child_body.is_empty() {
                        issue.notes = Some(unsanitize_org_text(&child_body));
                    }
                }
                "Dependencies" => {
                    // Extract JSON from src block
                    if let Some(json) = extract_json_from_body(&child.body) {
                        issue.dependencies = serde_json::from_str(&json)
                            .map_err(BeadsError::Json)?;
                    }
                }
                "Comments" => {
                    // Extract JSON from src block
                    if let Some(json) = extract_json_from_body(&child.body) {
                        issue.comments = serde_json::from_str(&json)
                            .map_err(BeadsError::Json)?;
                    }
                }
                _ => {
                    // Unknown child heading, ignore
                }
            }
        }
    }

    Ok(issue)
}

/// Extract plain text from inline content.
fn extract_title_text(contents: &[InlineContent]) -> String {
    let mut out = String::new();
    for item in contents {
        flatten_inline_to_text(item, &mut out);
    }
    out
}

/// Recursively flatten all inline content to plain text.
/// Handles subscripts, superscripts, bold, italic, etc. so that
/// titles like "BEADS_DIR" (where orgize parses `_DIR` as a subscript)
/// are reconstructed properly.
fn flatten_inline_to_text(item: &InlineContent, out: &mut String) {
    match item {
        InlineContent::Text { value }
        | InlineContent::LatexFragment { value }
        | InlineContent::Timestamp { value } => out.push_str(value),
        InlineContent::Subscript { contents, use_braces } => {
            out.push('_');
            if *use_braces {
                out.push('{');
            }
            for child in contents {
                flatten_inline_to_text(child, out);
            }
            if *use_braces {
                out.push('}');
            }
        }
        InlineContent::Superscript { contents, use_braces } => {
            out.push('^');
            if *use_braces {
                out.push('{');
            }
            for child in contents {
                flatten_inline_to_text(child, out);
            }
            if *use_braces {
                out.push('}');
            }
        }
        InlineContent::Bold { contents }
        | InlineContent::Italic { contents }
        | InlineContent::Underline { contents }
        | InlineContent::StrikeThrough { contents } => {
            for child in contents {
                flatten_inline_to_text(child, out);
            }
        }
        InlineContent::Code { value } | InlineContent::Verbatim { value } => {
            out.push_str(value);
        }
        InlineContent::LineBreak => out.push('\n'),
        InlineContent::Entity { name } => out.push_str(name),
        InlineContent::Link { description, path, .. } => {
            if let Some(desc_contents) = description {
                for child in desc_contents {
                    flatten_inline_to_text(child, out);
                }
            } else {
                out.push_str(path);
            }
        }
        _ => {} // Skip unknown variants
    }
}

/// Extract text from body elements (paragraphs).
fn extract_body_text(body: &[Element]) -> String {
    let mut text = String::new();

    for (i, element) in body.iter().enumerate() {
        if let Element::Paragraph { contents } = element {
            if i > 0 {
                text.push_str("\n\n");
            }
            let para_text = extract_title_text(contents);
            text.push_str(&para_text);
        }
    }

    text
}

/// Extract JSON content from a src block in the body.
fn extract_json_from_body(body: &[Element]) -> Option<String> {
    for element in body {
        if let Element::SrcBlock { language, value, .. } = element {
            if language == "json" {
                return Some(value.clone());
            }
        }
    }
    None
}

/// Un-escape Org text (convert `,*` back to `*`).
fn unsanitize_org_text(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.starts_with(",*") {
                line[1..].to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse properties from the property drawer into the Issue struct.
#[allow(clippy::too_many_lines)]
#[allow(clippy::wildcard_in_or_patterns)]
fn parse_properties(properties: &[org2jsonl::model::Property], issue: &mut Issue) -> Result<()> {
    for prop in properties {
        let key = prop.key.as_str();
        let value = prop.value.trim();

        match key {
            "ID" => {
                issue.id = value.to_string();
            }
            "ISSUE_TYPE" => {
                issue.issue_type = IssueType::from_str(value)?;
            }
            "CREATED_AT" => {
                issue.created_at = DateTime::parse_from_rfc3339(value)
                    .map_err(|e| BeadsError::Validation {
                        field: "created_at".to_string(),
                        reason: e.to_string(),
                    })?
                    .with_timezone(&Utc);
            }
            "UPDATED_AT" => {
                issue.updated_at = DateTime::parse_from_rfc3339(value)
                    .map_err(|e| BeadsError::Validation {
                        field: "updated_at".to_string(),
                        reason: e.to_string(),
                    })?
                    .with_timezone(&Utc);
            }
            "ASSIGNEE" => {
                issue.assignee = Some(value.to_string());
            }
            "OWNER" => {
                issue.owner = Some(value.to_string());
            }
            "ESTIMATED_MINUTES" => {
                issue.estimated_minutes = Some(value.parse().map_err(|e| {
                    BeadsError::Validation {
                        field: "estimated_minutes".to_string(),
                        reason: format!("Invalid integer: {e}"),
                    }
                })?);
            }
            "CREATED_BY" => {
                issue.created_by = Some(value.to_string());
            }
            "CLOSED_AT" => {
                issue.closed_at = Some(
                    DateTime::parse_from_rfc3339(value)
                        .map_err(|e| BeadsError::Validation {
                            field: "closed_at".to_string(),
                            reason: e.to_string(),
                        })?
                        .with_timezone(&Utc),
                );
            }
            "CLOSE_REASON" => {
                issue.close_reason = Some(value.to_string());
            }
            "CLOSED_BY_SESSION" => {
                issue.closed_by_session = Some(value.to_string());
            }
            "DUE_AT" => {
                issue.due_at = Some(
                    DateTime::parse_from_rfc3339(value)
                        .map_err(|e| BeadsError::Validation {
                            field: "due_at".to_string(),
                            reason: e.to_string(),
                        })?
                        .with_timezone(&Utc),
                );
            }
            "DEFER_UNTIL" => {
                issue.defer_until = Some(
                    DateTime::parse_from_rfc3339(value)
                        .map_err(|e| BeadsError::Validation {
                            field: "defer_until".to_string(),
                            reason: e.to_string(),
                        })?
                        .with_timezone(&Utc),
                );
            }
            "EXTERNAL_REF" => {
                issue.external_ref = Some(value.to_string());
            }
            "SOURCE_SYSTEM" => {
                issue.source_system = Some(value.to_string());
            }
            "SOURCE_REPO" => {
                issue.source_repo = Some(value.to_string());
            }
            "DELETED_AT" => {
                issue.deleted_at = Some(
                    DateTime::parse_from_rfc3339(value)
                        .map_err(|e| BeadsError::Validation {
                            field: "deleted_at".to_string(),
                            reason: e.to_string(),
                        })?
                        .with_timezone(&Utc),
                );
            }
            "DELETED_BY" => {
                issue.deleted_by = Some(value.to_string());
            }
            "DELETE_REASON" => {
                issue.delete_reason = Some(value.to_string());
            }
            "ORIGINAL_TYPE" => {
                issue.original_type = Some(value.to_string());
            }
            "COMPACTION_LEVEL" => {
                issue.compaction_level = Some(value.parse().map_err(|e| {
                    BeadsError::Validation {
                        field: "compaction_level".to_string(),
                        reason: format!("Invalid integer: {e}"),
                    }
                })?);
            }
            "COMPACTED_AT" => {
                issue.compacted_at = Some(
                    DateTime::parse_from_rfc3339(value)
                        .map_err(|e| BeadsError::Validation {
                            field: "compacted_at".to_string(),
                            reason: e.to_string(),
                        })?
                        .with_timezone(&Utc),
                );
            }
            "COMPACTED_AT_COMMIT" => {
                issue.compacted_at_commit = Some(value.to_string());
            }
            "ORIGINAL_SIZE" => {
                issue.original_size = Some(value.parse().map_err(|e| {
                    BeadsError::Validation {
                        field: "original_size".to_string(),
                        reason: format!("Invalid integer: {e}"),
                    }
                })?);
            }
            "SENDER" => {
                issue.sender = Some(value.to_string());
            }
            "EPHEMERAL" => {
                issue.ephemeral = value.eq_ignore_ascii_case("true");
            }
            "PINNED" => {
                issue.pinned = value.eq_ignore_ascii_case("true");
            }
            "IS_TEMPLATE" => {
                issue.is_template = value.eq_ignore_ascii_case("true");
            }
            "BEADS_SCHEMA_VERSION" | _ => {
                // BEADS_SCHEMA_VERSION is validated elsewhere if needed
                // Unknown properties are silently ignored
            }
        }
    }

    // Validate that required ID field is present
    if issue.id.is_empty() {
        return Err(BeadsError::Validation {
            field: "id".to_string(),
            reason: "Missing required :ID: property".to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_sanitize_org_text() {
        let text = "Normal line\n* Heading line\nAnother normal line";
        let sanitized = sanitize_org_text(text);
        assert_eq!(sanitized, "Normal line\n,* Heading line\nAnother normal line");
    }

    #[test]
    fn test_sanitize_property_value() {
        // Newlines are replaced with spaces
        assert_eq!(sanitize_property_value("line1\nline2"), "line1 line2");
        assert_eq!(sanitize_property_value("line1\r\nline2"), "line1  line2");
        // :END: token is defused
        assert_eq!(sanitize_property_value("foo:END:bar"), "foo:END bar");
        // Normal values pass through
        assert_eq!(sanitize_property_value("normal value"), "normal value");
    }

    #[test]
    fn test_unsanitize_org_text() {
        let text = "Normal line\n,* Heading line\nAnother normal line";
        let unsanitized = unsanitize_org_text(text);
        assert_eq!(unsanitized, "Normal line\n* Heading line\nAnother normal line");
    }

    #[test]
    fn test_status_keyword_roundtrip() {
        let statuses = vec![
            Status::Open,
            Status::InProgress,
            Status::Blocked,
            Status::Deferred,
            Status::Closed,
            Status::Tombstone,
            Status::Pinned,
        ];

        for status in statuses {
            let keyword = status_to_keyword(&status);
            let parsed = keyword_to_status(&keyword).unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[test]
    fn test_priority_roundtrip() {
        for i in 0..=4 {
            let priority = Priority(i);
            let org = priority_to_org(priority);
            let letter = org.chars().nth(2).unwrap().to_string();
            let parsed = org_to_priority(Some(&letter));
            assert_eq!(priority, parsed);
        }
    }

    #[test]
    fn test_issue_to_org_basic() {
        let issue = Issue {
            id: "bd-test".to_string(),
            title: "Test Issue".to_string(),
            description: Some("Test description".to_string()),
            status: Status::Open,
            priority: Priority::HIGH,
            issue_type: IssueType::Task,
            created_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            labels: vec!["test".to_string(), "demo".to_string()],
            ..Default::default()
        };

        let org_text = issues_to_org_text(&[issue]);

        assert!(org_text.contains("* OPEN [#B] Test Issue"));
        assert!(org_text.contains(":demo:test:"));
        assert!(org_text.contains(":ID:       bd-test"));
        assert!(org_text.contains("Test description"));
    }
}
