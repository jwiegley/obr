use beads_rust::model::{Issue, IssueType, Priority, Status};
use beads_rust::util::content_hash;
use chrono::Utc;

#[test]
fn test_hash_collision_null_byte_injection() {
    let mut issue1 = make_test_issue();
    // title ends with null
    issue1.title = "foo\0".to_string();
    issue1.description = Some("bar".to_string());

    let mut issue2 = make_test_issue();
    // title normal
    issue2.title = "foo".to_string();
    // description starts with null
    issue2.description = Some("\0bar".to_string());

    let hash1 = content_hash(&issue1);
    let hash2 = content_hash(&issue2);

    println!("Hash1: {hash1}");
    println!("Hash2: {hash2}");

    // These SHOULD be different for safety, but currently might collide
    assert_ne!(
        hash1, hash2,
        "Hash collision detected with null byte injection!"
    );
}

fn make_test_issue() -> Issue {
    Issue {
        id: "bd-test".to_string(),
        content_hash: None,
        title: "Test".to_string(),
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
