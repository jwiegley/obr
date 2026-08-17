#![no_main]

mod common;

use obr::model::{Comment, Dependency, DependencyType, Issue, Priority};
use obr::util::content_hash;
use obr::validation::IssueValidator;
use common::{ByteCursor, EmptyCustomIssueCursorExt};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use std::error::Error;

const MAX_INPUT_BYTES: usize = 4 * 1024;
const MAX_FIELD_BYTES: usize = 128;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }

    let result = run_hash_case(data);
    assert!(
        result.is_ok(),
        "content hash fuzz invariant failed: {result:?}"
    );
});

fn run_hash_case(data: &[u8]) -> Result<(), Box<dyn Error>> {
    let issue = issue_from_bytes(data);
    if IssueValidator::validate(&issue).is_err() {
        return Ok(());
    }

    let hash = content_hash(&issue);
    assert_hash_shape(&hash)?;

    let repeated_hash = content_hash(&issue);
    if hash != repeated_hash {
        return Err("content hash is not deterministic for identical input".into());
    }

    assert_json_formatting_stability(&issue, &hash, data)?;
    assert_unknown_json_fields_do_not_affect_hash(&issue, &hash, data)?;
    assert_empty_optional_equivalence(&issue, &hash)?;
    assert_nul_space_equivalence(&issue, &hash)?;
    assert_ignored_fields_do_not_affect_hash(&issue, &hash, data)?;
    assert_meaningful_change_affects_hash(&issue, &hash)?;

    Ok(())
}

fn issue_from_bytes(data: &[u8]) -> Issue {
    let mut cursor = ByteCursor::new(data);
    let mut issue = Issue {
        id: format!("fuzz-{}", cursor.next_byte()),
        title: non_empty(cursor.text(MAX_FIELD_BYTES), "fuzz-title"),
        description: cursor.optional_text(MAX_FIELD_BYTES),
        design: cursor.optional_text(MAX_FIELD_BYTES),
        acceptance_criteria: cursor.optional_text(MAX_FIELD_BYTES),
        notes: cursor.optional_text(MAX_FIELD_BYTES),
        status: cursor.status(),
        priority: Priority(i32::from(cursor.next_byte() % 5)),
        issue_type: cursor.issue_type(),
        assignee: cursor.optional_text(48),
        owner: cursor.optional_text(48),
        created_by: cursor.optional_text(48),
        external_ref: cursor.optional_text(64),
        source_system: cursor.optional_text(48),
        pinned: cursor.next_bool(),
        is_template: cursor.next_bool(),
        ..Issue::default()
    };

    issue.labels = vec![cursor.text(24), cursor.text(24)]
        .into_iter()
        .filter(|label| !label.is_empty())
        .collect();

    issue
}

fn assert_json_formatting_stability(
    issue: &Issue,
    expected_hash: &str,
    data: &[u8],
) -> Result<(), Box<dyn Error>> {
    let compact = serde_json::to_vec(issue)?;
    let compact_issue: Issue = serde_json::from_slice(&compact)?;
    assert_hash_eq(
        expected_hash,
        &content_hash(&compact_issue),
        "compact JSON round-trip changed content hash",
    )?;

    let pretty = serde_json::to_vec_pretty(issue)?;
    let pretty_issue: Issue = serde_json::from_slice(&pretty)?;
    assert_hash_eq(
        expected_hash,
        &content_hash(&pretty_issue),
        "pretty JSON round-trip changed content hash",
    )?;

    assert_randomized_json_formatting_stability(issue, expected_hash, data)
}

fn assert_randomized_json_formatting_stability(
    issue: &Issue,
    expected_hash: &str,
    data: &[u8],
) -> Result<(), Box<dyn Error>> {
    let value = serde_json::to_value(issue)?;
    let Value::Object(map) = value else {
        return Err("issue did not serialize to a JSON object".into());
    };

    let mut entries: Vec<(String, Value)> = map.into_iter().collect();
    reorder_entries(&mut entries, data);

    let rendered = render_json_object_with_spacing(&entries, data)?;
    let parsed: Issue = serde_json::from_str(&rendered)?;
    assert_hash_eq(
        expected_hash,
        &content_hash(&parsed),
        "randomized JSON field order/whitespace changed content hash",
    )?;

    let crlf_rendered = rendered.replace('\n', "\r\n");
    let crlf_parsed: Issue = serde_json::from_str(&crlf_rendered)?;
    assert_hash_eq(
        expected_hash,
        &content_hash(&crlf_parsed),
        "CRLF JSON formatting changed content hash",
    )
}

fn reorder_entries(entries: &mut [(String, Value)], data: &[u8]) {
    if entries.is_empty() {
        return;
    }

    entries.rotate_left(data.len() % entries.len());
    if data.first().copied().unwrap_or(0) & 1 == 1 {
        entries.reverse();
    }

    for (index, byte) in data.iter().copied().take(entries.len()).enumerate() {
        entries.swap(index, usize::from(byte) % entries.len());
    }
}

fn render_json_object_with_spacing(
    entries: &[(String, Value)],
    data: &[u8],
) -> Result<String, serde_json::Error> {
    let mut rendered = String::from("{");
    for (index, (key, value)) in entries.iter().enumerate() {
        push_json_spacing(&mut rendered, data, index * 5);
        rendered.push_str(&serde_json::to_string(key)?);
        push_json_spacing(&mut rendered, data, index * 5 + 1);
        rendered.push(':');
        push_json_spacing(&mut rendered, data, index * 5 + 2);
        rendered.push_str(&serde_json::to_string(value)?);

        if index + 1 != entries.len() {
            push_json_spacing(&mut rendered, data, index * 5 + 3);
            rendered.push(',');
        }
    }
    push_json_spacing(&mut rendered, data, entries.len() * 5);
    rendered.push('}');
    Ok(rendered)
}

fn push_json_spacing(rendered: &mut String, data: &[u8], salt: usize) {
    let byte = if data.is_empty() {
        salt.to_le_bytes()[0]
    } else {
        data[salt % data.len()]
    };

    match byte % 6 {
        0 => {}
        1 => rendered.push(' '),
        2 => rendered.push('\n'),
        3 => rendered.push_str("\n  "),
        4 => rendered.push('\t'),
        _ => rendered.push_str(" \n\t"),
    }
}

fn assert_unknown_json_fields_do_not_affect_hash(
    issue: &Issue,
    expected_hash: &str,
    data: &[u8],
) -> Result<(), Box<dyn Error>> {
    let mut value = serde_json::to_value(issue)?;
    if let Value::Object(map) = &mut value {
        map.insert(
            "unknown_fuzz_field".to_string(),
            Value::String(String::from_utf8_lossy(data).into_owned()),
        );
        map.insert(
            "unknown_fuzz_array".to_string(),
            Value::Array(vec![Value::Bool(true)]),
        );
    }

    let parsed: Issue = serde_json::from_value(value)?;
    assert_hash_eq(
        expected_hash,
        &content_hash(&parsed),
        "unknown JSON fields changed content hash",
    )
}

fn assert_empty_optional_equivalence(
    issue: &Issue,
    expected_hash: &str,
) -> Result<(), Box<dyn Error>> {
    let mut variant = issue.clone();
    flip_empty_optional(&mut variant.description);
    flip_empty_optional(&mut variant.design);
    flip_empty_optional(&mut variant.acceptance_criteria);
    flip_empty_optional(&mut variant.notes);
    flip_empty_optional(&mut variant.assignee);
    flip_empty_optional(&mut variant.owner);
    flip_empty_optional(&mut variant.created_by);
    flip_empty_optional(&mut variant.external_ref);
    flip_empty_optional(&mut variant.source_system);

    assert_hash_eq(
        expected_hash,
        &content_hash(&variant),
        "empty optional field equivalence changed content hash",
    )
}

fn assert_nul_space_equivalence(issue: &Issue, expected_hash: &str) -> Result<(), Box<dyn Error>> {
    let mut variant = issue.clone();
    replace_nul_with_space(&mut variant.title);
    replace_optional_nul_with_space(&mut variant.description);
    replace_optional_nul_with_space(&mut variant.design);
    replace_optional_nul_with_space(&mut variant.acceptance_criteria);
    replace_optional_nul_with_space(&mut variant.notes);
    replace_optional_nul_with_space(&mut variant.assignee);
    replace_optional_nul_with_space(&mut variant.owner);
    replace_optional_nul_with_space(&mut variant.created_by);
    replace_optional_nul_with_space(&mut variant.external_ref);
    replace_optional_nul_with_space(&mut variant.source_system);

    assert_hash_eq(
        expected_hash,
        &content_hash(&variant),
        "NUL-to-space canonicalization changed content hash",
    )
}

fn assert_ignored_fields_do_not_affect_hash(
    issue: &Issue,
    expected_hash: &str,
    data: &[u8],
) -> Result<(), Box<dyn Error>> {
    let mut variant = issue.clone();
    let created_at = variant.created_at;

    variant.id = format!("ignored-{}", data.len());
    variant.content_hash = Some(String::from("stale-hash"));
    variant.updated_at = created_at;
    variant.closed_at = Some(created_at);
    variant.close_reason = Some(String::from("ignored close reason"));
    variant.closed_by_session = Some(String::from("ignored-session"));
    variant.estimated_minutes = Some(i32::from(data.first().copied().unwrap_or(0)));
    variant.deleted_at = Some(created_at);
    variant.deleted_by = Some(String::from("ignored deleter"));
    variant.delete_reason = Some(String::from("ignored delete reason"));
    variant.original_type = Some(String::from("ignored original type"));
    variant.compaction_level = Some(7);
    variant.compacted_at = Some(created_at);
    variant.compacted_at_commit = Some(String::from("ignored-commit"));
    variant.original_size = Some(123);
    variant.sender = Some(String::from("ignored sender"));
    variant.ephemeral = !variant.ephemeral;
    variant.labels.reverse();
    variant.labels.push(String::from("ignored-label"));
    variant.dependencies.push(Dependency {
        issue_id: variant.id.clone(),
        depends_on_id: String::from("external:fuzz:ignored"),
        dep_type: DependencyType::Blocks,
        created_at,
        created_by: Some(String::from("ignored")),
        metadata: None,
        thread_id: Some(String::from("ignored-thread")),
    });
    variant.comments.push(Comment {
        id: 1,
        issue_id: variant.id.clone(),
        author: String::from("ignored"),
        body: String::from("ignored comment"),
        created_at,
    });

    assert_hash_eq(
        expected_hash,
        &content_hash(&variant),
        "ignored issue fields changed content hash",
    )
}

fn assert_meaningful_change_affects_hash(
    issue: &Issue,
    expected_hash: &str,
) -> Result<(), Box<dyn Error>> {
    let mut changed = issue.clone();
    changed.title.push_str("\u{1f}meaningful");
    let changed_hash = content_hash(&changed);
    assert_hash_shape(&changed_hash)?;
    if changed_hash == expected_hash {
        return Err("title change did not affect content hash".into());
    }

    let mut formatting_changed = issue.clone();
    match &mut formatting_changed.description {
        Some(description) => description.push('\n'),
        None => formatting_changed.description = Some(String::from("\n")),
    }
    let formatting_hash = content_hash(&formatting_changed);
    assert_hash_shape(&formatting_hash)?;
    if formatting_hash == expected_hash {
        return Err("included text formatting change did not affect content hash".into());
    }

    Ok(())
}

fn assert_hash_shape(hash: &str) -> Result<(), Box<dyn Error>> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(format!("content hash is not 64 lowercase hex chars: {hash}").into());
    }
    Ok(())
}

fn assert_hash_eq(expected: &str, actual: &str, context: &str) -> Result<(), Box<dyn Error>> {
    assert_hash_shape(actual)?;
    if expected != actual {
        return Err(format!("{context}: expected {expected}, got {actual}").into());
    }
    Ok(())
}

fn flip_empty_optional(field: &mut Option<String>) {
    match field {
        None => *field = Some(String::new()),
        Some(value) if value.is_empty() => *field = None,
        Some(_) => {}
    }
}

fn replace_optional_nul_with_space(field: &mut Option<String>) {
    if let Some(value) = field {
        replace_nul_with_space(value);
    }
}

fn replace_nul_with_space(value: &mut String) {
    if value.contains('\0') {
        *value = value.replace('\0', " ");
    }
}

fn non_empty(value: String, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}
