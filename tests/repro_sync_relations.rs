mod common;

use common::fixtures;
use obr::storage::SqliteStorage;
use obr::sync::{ExportConfig, ImportConfig, export_to_jsonl, import_from_jsonl};
use tempfile::TempDir;

#[test]
fn test_relation_updates_bump_timestamp_and_sync() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("issues.jsonl");

    // 1. Setup Source DB
    let mut source_db = SqliteStorage::open_memory().unwrap();
    let issue = fixtures::issue("Test Issue");
    source_db.create_issue(&issue, "tester").unwrap();

    // Initial export
    export_to_jsonl(&source_db, &path, &ExportConfig::default()).unwrap();

    // 2. Setup Target DB (simulating another machine)
    let mut target_db = SqliteStorage::open_memory().unwrap();
    import_from_jsonl(
        &mut target_db,
        &path,
        &ImportConfig::default(),
        Some("test-"),
    )
    .unwrap();

    // Verify sync baseline
    let _target_issue = target_db.get_issue(&issue.id).unwrap().unwrap();
    let target_labels = target_db.get_labels(&issue.id).unwrap();
    assert!(target_labels.is_empty());

    // 3. Modify Source: Add Label
    // Sleep to ensure timestamp would advance if we updated it
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Check timestamp before
    let before_update = source_db.get_issue(&issue.id).unwrap().unwrap().updated_at;

    source_db.add_label(&issue.id, "bug", "tester").unwrap();

    // Check timestamp after
    let after_update = source_db.get_issue(&issue.id).unwrap().unwrap().updated_at;

    // ASSERTION 1: Timestamp should change
    assert!(
        after_update > before_update,
        "Adding label should update issue timestamp"
    );

    // 4. Sync Source -> Target
    export_to_jsonl(&source_db, &path, &ExportConfig::default()).unwrap();

    let import_result = import_from_jsonl(
        &mut target_db,
        &path,
        &ImportConfig::default(),
        Some("test-"),
    )
    .unwrap();

    // ASSERTION 2: Import should update, not skip
    assert_eq!(
        import_result.imported_count, 1,
        "Should import updated issue"
    );
    assert_eq!(
        import_result.skipped_count, 0,
        "Should not skip updated issue"
    );

    // ASSERTION 3: Label should be present in target
    let target_labels = target_db.get_labels(&issue.id).unwrap();
    assert_eq!(target_labels, vec!["bug".to_string()]);
}

#[test]
fn test_dependency_updates_bump_timestamp() {
    let mut db = SqliteStorage::open_memory().unwrap();
    let issue1 = fixtures::issue("Issue 1");
    let issue2 = fixtures::issue("Issue 2");
    db.create_issue(&issue1, "tester").unwrap();
    db.create_issue(&issue2, "tester").unwrap();

    let before = db.get_issue(&issue1.id).unwrap().unwrap().updated_at;
    std::thread::sleep(std::time::Duration::from_millis(100));

    db.add_dependency(&issue1.id, &issue2.id, "blocks", "tester")
        .unwrap();

    let after = db.get_issue(&issue1.id).unwrap().unwrap().updated_at;
    assert!(
        after > before,
        "Adding dependency should update issue timestamp"
    );
}
