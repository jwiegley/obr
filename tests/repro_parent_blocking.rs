use chrono::Utc;
use obr::model::{DependencyType, Issue, IssueType};
use obr::storage::{ReadyFilters, ReadySortPolicy, SqliteStorage};

#[test]
fn test_blocked_parent_blocks_child_ready() {
    let mut storage = SqliteStorage::open_memory().unwrap();
    let now = Utc::now();

    let blocker = Issue {
        id: "bd-blocker".to_string(),
        title: "Root blocker".to_string(),
        issue_type: IssueType::Bug,
        created_at: now,
        updated_at: now,
        created_by: Some("tester".to_string()),
        ..Issue::default()
    };
    storage.create_issue(&blocker, "tester").unwrap();

    // Create a parent issue
    let parent = Issue {
        id: "bd-parent".to_string(),
        title: "Parent Epic".to_string(),
        issue_type: IssueType::Epic,
        created_at: now,
        updated_at: now,
        created_by: Some("tester".to_string()),
        ..Issue::default()
    };
    storage.create_issue(&parent, "tester").unwrap();

    // Create a child issue
    let child = Issue {
        id: "bd-child".to_string(),
        title: "Child Task".to_string(),
        issue_type: IssueType::Task,
        created_at: now,
        updated_at: now,
        created_by: Some("tester".to_string()),
        ..Issue::default()
    };
    storage.create_issue(&child, "tester").unwrap();

    storage
        .add_dependency(
            "bd-parent",
            "bd-blocker",
            DependencyType::Blocks.as_str(),
            "tester",
        )
        .unwrap();

    // Parent-child relationships should propagate a blocked parent to its child.
    storage
        .add_dependency(
            "bd-child",
            "bd-parent",
            DependencyType::ParentChild.as_str(),
            "tester",
        )
        .unwrap();

    // Manually trigger cache rebuild
    storage.rebuild_blocked_cache(true).unwrap();

    assert!(storage.is_blocked("bd-parent").unwrap());

    // Check if child is blocked transitively through the parent.
    let is_blocked = storage.is_blocked("bd-child").unwrap();
    assert!(is_blocked, "Child should be blocked by blocked parent");

    // Check if child is in ready issues
    let ready = storage
        .get_ready_issues(&ReadyFilters::default(), ReadySortPolicy::default())
        .unwrap();
    let is_ready = ready.iter().any(|i| i.id == "bd-child");
    assert!(
        !is_ready,
        "Child should not be ready when its parent is blocked"
    );
}
