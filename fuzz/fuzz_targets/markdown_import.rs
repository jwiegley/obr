#![no_main]

use obr::model::{Issue, IssueType, Priority, Status};
use obr::util::markdown_import::{ParsedIssue, parse_markdown_content};
use obr::validation::IssueValidator;
use chrono::{DateTime, Utc};
use libfuzzer_sys::fuzz_target;
use std::error::Error;
use std::str::FromStr;

const MAX_INPUT_BYTES: usize = 32 * 1024;
const ACTOR: &str = "markdown-import-fuzz";

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }

    let result = run_markdown_import_case(data);
    assert!(
        result.is_ok(),
        "markdown import fuzz invariant failed: {result:?}"
    );
});

fn run_markdown_import_case(data: &[u8]) -> Result<(), Box<dyn Error>> {
    let content = String::from_utf8_lossy(data);

    match parse_markdown_content(&content) {
        Ok(parsed_issues) => {
            for (index, parsed) in parsed_issues.iter().enumerate() {
                if let Some(mut issue) = validation_candidate(parsed, index)? {
                    issue.content_hash = Some(issue.compute_content_hash());
                    IssueValidator::validate(&issue).map_err(|errors| {
                        format!("parsed markdown issue failed validation: {errors:?}")
                    })?;
                }
            }
        }
        Err(err) => {
            let message = err.to_string();
            if message.trim().is_empty() {
                return Err("markdown parser returned an empty error message".into());
            }
        }
    }

    Ok(())
}

fn validation_candidate(
    parsed: &ParsedIssue,
    index: usize,
) -> Result<Option<Issue>, Box<dyn Error>> {
    let title = parsed.title.trim();
    if title.is_empty() || title.len() > 500 {
        return Ok(None);
    }

    let priority = match parsed.priority.as_deref() {
        Some(raw) => match Priority::from_str(raw) {
            Ok(priority) => priority,
            Err(_) => return Ok(None),
        },
        None => Priority::MEDIUM,
    };

    let issue_type = match parsed.issue_type.as_deref() {
        Some(raw) => IssueType::from_str(raw)?,
        None => IssueType::Task,
    };

    let description = parsed.description.clone();
    if description
        .as_ref()
        .is_some_and(|value| value.len() > 102_400)
    {
        return Ok(None);
    }

    let now = timestamp_for(index)?;
    Ok(Some(Issue {
        id: format!("fuzz-{index:x}"),
        title: title.to_string(),
        description,
        design: parsed.design.clone(),
        acceptance_criteria: parsed.acceptance_criteria.clone(),
        status: Status::Open,
        priority,
        issue_type,
        assignee: parsed.assignee.clone(),
        created_at: now,
        updated_at: now,
        created_by: Some(ACTOR.to_string()),
        ..Issue::default()
    }))
}

fn timestamp_for(index: usize) -> Result<DateTime<Utc>, Box<dyn Error>> {
    let offset = i64::try_from(index)?;
    DateTime::from_timestamp(1_700_000_000_i64.saturating_add(offset), 0)
        .ok_or_else(|| "failed to construct fuzz timestamp".into())
}
