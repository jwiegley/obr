mod common;

use common::cli::{BrWorkspace, extract_json_payload, run_br};
use serde_json::Value;

#[test]
fn test_create_json_output_includes_labels_and_deps() {
    let workspace = BrWorkspace::new();

    // Init
    let init = run_br(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Create blocking issue first
    let blocker = run_br(
        &workspace,
        ["create", "Blocker", "--json"],
        "create_blocker",
    );
    assert!(
        blocker.status.success(),
        "Failed to create blocking issue: {}",
        blocker.stderr
    );

    let blocker_json: Value = serde_json::from_str(&extract_json_payload(&blocker.stdout)).unwrap();
    let blocker_id = blocker_json["id"].as_str().unwrap();

    // Create issue with label and dep
    let issue = run_br(
        &workspace,
        [
            "create", "My Issue", "--labels", "bug", "--deps", blocker_id, "--json",
        ],
        "create_issue",
    );
    assert!(
        issue.status.success(),
        "Failed to create issue with label and dep: {}",
        issue.stderr
    );

    let issue_json: Value = serde_json::from_str(&extract_json_payload(&issue.stdout)).unwrap();
    // Verify fields
    let labels = issue_json["labels"]
        .as_array()
        .expect("labels should be an array");
    let deps = issue_json["dependencies"]
        .as_array()
        .expect("dependencies should be an array");

    assert!(
        labels.iter().any(|l| l.as_str() == Some("bug")),
        "Labels should contain 'bug'"
    );
    assert!(
        deps.iter()
            .any(|d| d["depends_on_id"].as_str() == Some(blocker_id)),
        "Dependencies should contain blocker ID"
    );
}
