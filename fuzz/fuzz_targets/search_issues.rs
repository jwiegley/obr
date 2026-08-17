#![no_main]

use obr::model::{Issue, IssueType, Priority, Status};
use obr::storage::{ListFilters, SqliteStorage};
use obr::validation::IssueValidator;
use chrono::{DateTime, Utc};
use libfuzzer_sys::fuzz_target;
use std::cell::OnceCell;
use std::collections::HashSet;
use std::error::Error;
use tempfile::{Builder, TempDir};

const MAX_INPUT_BYTES: usize = 16 * 1024;
const ACTOR: &str = "search-issues-fuzz";

thread_local! {
    static HARNESS: OnceCell<SearchHarness> = const { OnceCell::new() };
}

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }

    let result = run_search_case(data);
    assert!(
        result.is_ok(),
        "search issues fuzz invariant failed: {result:?}"
    );
});

fn run_search_case(data: &[u8]) -> Result<(), Box<dyn Error>> {
    let query = String::from_utf8_lossy(data);
    HARNESS.with(|cell| {
        if cell.get().is_none() {
            cell.set(SearchHarness::new()?)
                .map_err(|_| "search harness already initialized".to_string())?;
        }

        let harness = cell
            .get()
            .ok_or_else(|| "search harness was not initialized".to_string())?;
        harness.assert_search(query.as_ref())
    })
}

struct SearchHarness {
    _temp: TempDir,
    storage: SqliteStorage,
    seeded_ids: HashSet<String>,
}

impl SearchHarness {
    fn new() -> Result<Self, Box<dyn Error>> {
        let temp = Builder::new().prefix("br-search-issues-fuzz").tempdir()?;
        let db_path = temp.path().join("obr.db");
        let mut storage = SqliteStorage::open(&db_path)?;
        let seeded_ids = seed_storage(&mut storage)?;

        Ok(Self {
            _temp: temp,
            storage,
            seeded_ids,
        })
    }

    fn assert_search(&self, query: &str) -> Result<(), Box<dyn Error>> {
        match self.storage.search_issues(query, &ListFilters::default()) {
            Ok(issues) => {
                for issue in issues {
                    if !self.seeded_ids.contains(&issue.id) {
                        return Err(format!("search returned phantom issue: {}", issue.id).into());
                    }
                    IssueValidator::validate(&issue)
                        .map_err(|errors| format!("search result failed validation: {errors:?}"))?;
                }
            }
            Err(err) => {
                let message = err.to_string();
                if message.trim().is_empty() {
                    return Err("search returned an empty error message".into());
                }
            }
        }

        Ok(())
    }
}

fn seed_storage(storage: &mut SqliteStorage) -> Result<HashSet<String>, Box<dyn Error>> {
    let issues = [
        issue(
            "fuzz-alpha",
            "Search handles alpha issue",
            "Plain description with percent signs, underscores, and backslashes.",
            Priority::HIGH,
        )?,
        issue(
            "fuzz-beta",
            "Unicode search surface",
            "Emoji, accents, CJK, and mixed whitespace should remain searchable.",
            Priority::MEDIUM,
        )?,
        issue(
            "fuzz-gamma",
            "SQL shaped input stays data",
            "Quotes, comment markers, wildcard tokens, and NUL-adjacent text are query input.",
            Priority::LOW,
        )?,
    ];

    let mut ids = HashSet::new();
    for issue in issues {
        ids.insert(issue.id.clone());
        storage.create_issue(&issue, ACTOR)?;
    }

    Ok(ids)
}

fn issue(
    id: &str,
    title: &str,
    description: &str,
    priority: Priority,
) -> Result<Issue, Box<dyn Error>> {
    let now = timestamp()?;
    let mut issue = Issue {
        id: id.to_string(),
        title: title.to_string(),
        description: Some(description.to_string()),
        status: Status::Open,
        priority,
        issue_type: IssueType::Task,
        created_at: now,
        updated_at: now,
        created_by: Some(ACTOR.to_string()),
        ..Issue::default()
    };
    issue.content_hash = Some(issue.compute_content_hash());
    IssueValidator::validate(&issue)
        .map_err(|errors| format!("seed issue failed validation: {errors:?}"))?;
    Ok(issue)
}

fn timestamp() -> Result<DateTime<Utc>, Box<dyn Error>> {
    DateTime::from_timestamp(1_700_000_000, 0)
        .ok_or_else(|| "failed to construct fuzz timestamp".into())
}
