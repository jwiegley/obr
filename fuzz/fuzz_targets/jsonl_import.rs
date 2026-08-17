#![no_main]

use obr::model::{DependencyType, Issue};
use obr::storage::SqliteStorage;
use obr::sync::{ImportConfig, import_from_jsonl};
use obr::validation::IssueValidator;
use libfuzzer_sys::fuzz_target;
use std::collections::HashSet;
use std::error::Error;
use std::fs;
use tempfile::Builder;

const MAX_INPUT_BYTES: usize = 64 * 1024;
const SENTINEL_PARENT_ID: &str = "fuzz-parent";
const SENTINEL_CHILD_ID: &str = "fuzz-existing";
const ACTOR: &str = "jsonl-import-fuzz";

#[derive(Debug, Eq, PartialEq)]
struct SentinelSnapshot {
    parent_title: String,
    child_title: String,
    labels: Vec<String>,
    dependencies: Vec<String>,
    comments: Vec<String>,
}

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }

    let result = run_import_case(data);
    assert!(
        result.is_ok(),
        "jsonl import fuzz harness invariant failed: {result:?}"
    );
});

fn run_import_case(data: &[u8]) -> Result<(), Box<dyn Error>> {
    let temp = Builder::new().prefix("br-jsonl-import-fuzz").tempdir()?;
    let obr_dir = temp.path().join(".obr");
    fs::create_dir(&obr_dir)?;

    let input_path = obr_dir.join("issues.jsonl");
    fs::write(&input_path, data)?;

    let db_path = obr_dir.join("obr.db");
    {
        let mut storage = SqliteStorage::open(&db_path)?;
        seed_sentinel_data(&mut storage)?;
        let before = sentinel_snapshot(&storage)?;

        let config = ImportConfig {
            skip_prefix_validation: true,
            clear_duplicate_external_refs: true,
            obr_dir: Some(obr_dir.clone()),
            show_progress: false,
            ..ImportConfig::default()
        };

        let import_result = import_from_jsonl(&mut storage, &input_path, &config, Some("fuzz-"));
        match import_result {
            Ok(_) => {
                assert_storage_invariants(&storage)?;
                assert_import_validation(&storage)?;
            }
            Err(err) => {
                let message = err.to_string();
                if message.trim().is_empty() {
                    return Err("import returned an empty error message".into());
                }

                let after = sentinel_snapshot(&storage)?;
                if before != after {
                    return Err(format!(
                        "failed import mutated existing storage: before={before:?} after={after:?}"
                    )
                    .into());
                }
                assert_storage_invariants(&storage)?;
            }
        }
    }

    let reopened = SqliteStorage::open(&db_path)?;
    assert_storage_invariants(&reopened)?;

    Ok(())
}

fn seed_sentinel_data(storage: &mut SqliteStorage) -> Result<(), Box<dyn Error>> {
    let mut parent = sentinel_issue(SENTINEL_PARENT_ID, "Sentinel parent");
    parent.external_ref = Some("external-parent".to_string());
    storage.create_issue(&parent, ACTOR)?;

    let mut child = sentinel_issue(SENTINEL_CHILD_ID, "Sentinel child");
    child.external_ref = Some("external-child".to_string());
    storage.create_issue(&child, ACTOR)?;

    storage.add_label(SENTINEL_CHILD_ID, "sentinel", ACTOR)?;
    storage.add_dependency(
        SENTINEL_CHILD_ID,
        SENTINEL_PARENT_ID,
        DependencyType::Blocks.as_str(),
        ACTOR,
    )?;
    storage.add_comment(SENTINEL_CHILD_ID, ACTOR, "sentinel comment")?;

    Ok(())
}

fn sentinel_issue(id: &str, title: &str) -> Issue {
    Issue {
        id: id.to_string(),
        title: title.to_string(),
        created_by: Some(ACTOR.to_string()),
        ..Issue::default()
    }
}

fn sentinel_snapshot(storage: &SqliteStorage) -> Result<SentinelSnapshot, Box<dyn Error>> {
    let parent = storage
        .get_issue(SENTINEL_PARENT_ID)?
        .ok_or("sentinel parent issue missing")?;
    let child = storage
        .get_issue(SENTINEL_CHILD_ID)?
        .ok_or("sentinel child issue missing")?;
    let labels = storage.get_labels(SENTINEL_CHILD_ID)?;
    let dependencies = storage.get_dependencies(SENTINEL_CHILD_ID)?;
    let comments = storage
        .get_comments(SENTINEL_CHILD_ID)?
        .into_iter()
        .map(|comment| comment.body)
        .collect();

    Ok(SentinelSnapshot {
        parent_title: parent.title,
        child_title: child.title,
        labels,
        dependencies,
        comments,
    })
}

fn assert_storage_invariants(storage: &SqliteStorage) -> Result<(), Box<dyn Error>> {
    let ids: HashSet<String> = storage.get_all_ids()?.into_iter().collect();

    for issue in storage.get_all_issues_for_export()? {
        if !ids.contains(&issue.id) {
            return Err(format!("export returned issue absent from id index: {}", issue.id).into());
        }
    }

    for (issue_id, labels) in storage.get_all_labels()? {
        if !ids.contains(&issue_id) {
            return Err(format!("label row references missing issue: {issue_id}").into());
        }
        let mut unique_labels = HashSet::new();
        for label in labels {
            if !unique_labels.insert(label.clone()) {
                return Err(format!("duplicate label for {issue_id}: {label}").into());
            }
        }
    }

    for (issue_id, comments) in storage.get_all_comments()? {
        if !ids.contains(&issue_id) {
            return Err(format!("comment row references missing issue: {issue_id}").into());
        }
        for comment in comments {
            if comment.issue_id != issue_id {
                return Err(format!(
                    "comment map key {issue_id} disagrees with comment issue_id {}",
                    comment.issue_id
                )
                .into());
            }
        }
    }

    for (issue_id, dependencies) in storage.get_all_dependency_records()? {
        if !ids.contains(&issue_id) {
            return Err(
                format!("dependency row references missing source issue: {issue_id}").into(),
            );
        }
        let mut unique_edges = HashSet::new();
        for dependency in dependencies {
            if dependency.issue_id != issue_id {
                return Err(format!(
                    "dependency map key {issue_id} disagrees with dependency issue_id {}",
                    dependency.issue_id
                )
                .into());
            }
            if !dependency.depends_on_id.starts_with("external:")
                && !ids.contains(&dependency.depends_on_id)
            {
                return Err(format!(
                    "dependency from {issue_id} references missing target {}",
                    dependency.depends_on_id
                )
                .into());
            }
            let edge = (dependency.issue_id, dependency.depends_on_id);
            if !unique_edges.insert(edge.clone()) {
                return Err(format!("duplicate dependency edge: {edge:?}").into());
            }
        }
    }

    Ok(())
}

fn assert_import_validation(storage: &SqliteStorage) -> Result<(), Box<dyn Error>> {
    for issue in storage.get_all_issues_for_export()? {
        if let Err(errors) = IssueValidator::validate(&issue) {
            // The import pipeline intentionally accepts data from external
            // systems (Go bd, manual JSONL, etc.) that may not conform to
            // all IssueValidator rules. Filter to violations that indicate
            // corrupted import behavior, not just permissive acceptance.
            // Skipped: id (external IDs), priority (not clamped on import),
            //   updated_at/closed_at (external timestamps), external_ref.
            let import_violations: Vec<_> = errors
                .into_iter()
                .filter(|e| {
                    matches!(
                        e.field.as_str(),
                        "title" | "description" | "estimated_minutes"
                    )
                })
                .collect();
            if !import_violations.is_empty() {
                return Err(format!(
                    "imported issue {} has validation failures: {import_violations:?}",
                    issue.id
                )
                .into());
            }
        }
    }
    Ok(())
}
