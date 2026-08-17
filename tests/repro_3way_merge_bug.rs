//! Reproduction of 3-way merge data loss bug.
//!
//! This test demonstrates that changes to labels, dependencies, or comments
//! must participate in merge-change detection even when scalar issue content
//! is unchanged.

mod common;

use common::cli::{ObrWorkspace, pin_jsonl, run_obr};
use std::fs;

fn create_issue_id(workspace: &ObrWorkspace, title: &str, label: &str) -> String {
    let create = run_obr(workspace, ["--json", "create", title, "-t", "task"], label);
    assert!(create.status.success(), "create failed: {}", create.stderr);

    let created_issue: serde_json::Value =
        serde_json::from_str(&create.stdout).expect("parse create json");
    created_issue["id"]
        .as_str()
        .expect("create json should include issue id")
        .to_string()
}

#[test]
fn repro_3way_merge_data_loss() {
    let workspace = ObrWorkspace::new();

    // 1. Initialize beads
    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed");
    pin_jsonl(&workspace.root.join(".obr"));

    // 2. Create an issue
    let issue_id = create_issue_id(&workspace, "Test issue", "create");

    // 3. Sync to JSONL (creates base snapshot)
    let sync1 = run_obr(&workspace, ["sync", "--flush-only"], "sync1");
    assert!(sync1.status.success(), "sync1 failed");
    let obr_dir = workspace.root.join(".obr");
    let jsonl_path = obr_dir.join("issues.jsonl");
    let base_snapshot_path = obr_dir.join("merge.base.jsonl");
    let base_jsonl = fs::read_to_string(&jsonl_path).expect("read base jsonl");
    fs::write(&base_snapshot_path, &base_jsonl).expect("seed base snapshot");

    // 4. Modify labels LOCALLY (Left side of merge), keeping JSONL at the base
    // snapshot so this is a true local-only relation change.
    let label_local = run_obr(
        &workspace,
        ["--no-auto-flush", "label", "add", &issue_id, "local-tag"],
        "label_local",
    );
    assert!(
        label_local.status.success(),
        "label_local failed: {}",
        label_local.stderr
    );

    // 5. Modify description EXTERNALLY (Right side of merge)
    // We simulate this by directly editing the JSONL.
    let jsonl_content = fs::read_to_string(&jsonl_path).expect("read jsonl");
    let mut issue: serde_json::Value =
        serde_json::from_str(jsonl_content.trim()).expect("parse jsonl");

    // Change description - this WILL change the content_hash
    issue["description"] = serde_json::Value::String("External description".to_string());
    issue["updated_at"] = serde_json::Value::String("2999-01-01T00:00:00Z".to_string());

    let modified_jsonl = serde_json::to_string(&issue).expect("serialize modified issue");
    fs::write(&jsonl_path, format!("{}\n", modified_jsonl)).expect("write modified jsonl");

    // 6. Run 3-way merge
    // At this point:
    // Base: labels=[], desc=""
    // Left (DB): labels=["local-tag"], desc=""
    // Right (JSONL): labels=[], desc="External description"

    let merge = run_obr(&workspace, ["sync", "--merge"], "merge");
    assert!(
        !merge.status.success(),
        "manual merge should stop instead of choosing a lossy side: stdout={} stderr={}",
        merge.stdout,
        merge.stderr
    );
    assert!(
        merge.stderr.contains("BothModified"),
        "manual merge should report a both-modified conflict: {}",
        merge.stderr
    );

    // 7. Verify the failed merge preserved both sides for explicit operator
    // resolution: DB still has the local tag, JSONL still has the external
    // description.
    let show = run_obr(
        &workspace,
        ["--allow-stale", "show", &issue_id, "--json"],
        "show_after_conflict",
    );
    assert!(show.status.success(), "show failed: {}", show.stderr);
    let final_issue_list: serde_json::Value =
        serde_json::from_str(&show.stdout).expect("parse final issue list");
    let final_issue = &final_issue_list[0];

    let labels = final_issue["labels"]
        .as_array()
        .expect("labels should be array");
    let has_local_tag = labels.iter().any(|v| v.as_str() == Some("local-tag"));

    assert!(
        has_local_tag,
        "DATA LOSS: Local tag 'local-tag' was lost during failed manual merge!\n\
         Final labels: {:?}",
        labels
    );

    let preserved_jsonl = fs::read_to_string(&jsonl_path).expect("read preserved jsonl");
    assert!(
        preserved_jsonl.contains("External description"),
        "DATA LOSS: External description was lost during failed manual merge!\n\
         Final JSONL: {preserved_jsonl}"
    );
}

#[test]
fn repro_merge_base_snapshot_matches_finalized_export_with_notes() {
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init_base_finalized");
    assert!(init.status.success(), "init failed: {}", init.stderr);
    pin_jsonl(&workspace.root.join(".obr"));

    let issue_id = create_issue_id(
        &workspace,
        "Base snapshot finality",
        "create_base_finalized",
    );

    let flush = run_obr(&workspace, ["sync", "--flush-only"], "flush_base_finalized");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let obr_dir = workspace.root.join(".obr");
    let jsonl_path = obr_dir.join("issues.jsonl");
    let base_snapshot_path = obr_dir.join("merge.base.jsonl");
    let original_jsonl = fs::read_to_string(&jsonl_path).expect("read original jsonl");
    fs::write(&base_snapshot_path, &original_jsonl).expect("seed base snapshot");

    let local_update = run_obr(
        &workspace,
        [
            "update",
            &issue_id,
            "--description",
            "Local description",
            "--no-auto-flush",
        ],
        "local_update_base_finalized",
    );
    assert!(
        local_update.status.success(),
        "local update failed: {}",
        local_update.stderr
    );

    let mut external_issue: serde_json::Value =
        serde_json::from_str(original_jsonl.trim()).expect("parse original issue");
    external_issue["description"] = serde_json::Value::String("External description".to_string());
    external_issue["updated_at"] = serde_json::Value::String("2999-01-01T00:00:00Z".to_string());
    fs::write(
        &jsonl_path,
        format!("{}\n", serde_json::to_string(&external_issue).unwrap()),
    )
    .expect("write external jsonl");

    let merge = run_obr(
        &workspace,
        ["sync", "--merge", "--force"],
        "merge_base_finalized",
    );
    assert!(merge.status.success(), "merge failed: {}", merge.stderr);

    let merged_jsonl = fs::read_to_string(&jsonl_path).expect("read merged jsonl");
    let base_jsonl = fs::read_to_string(&base_snapshot_path).expect("read base snapshot");
    assert_eq!(
        base_jsonl, merged_jsonl,
        "base snapshot should be copied from the finalized exported JSONL"
    );

    let base_issue: serde_json::Value =
        serde_json::from_str(base_jsonl.trim()).expect("parse base issue");
    let comments = base_issue["comments"]
        .as_array()
        .expect("base snapshot comments should be an array");
    assert!(
        comments.iter().any(|comment| {
            comment["author"].as_str() == Some("br-sync")
                && comment["text"]
                    .as_str()
                    .is_some_and(|body| body.contains("Both modified"))
        }),
        "base snapshot should include merge note comment from finalized export: {base_issue}"
    );
}

#[test]
fn repro_merge_tolerates_base_only_deleted_issue_absent_from_db() {
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);
    pin_jsonl(&workspace.root.join(".obr"));

    let issue_id = create_issue_id(&workspace, "Current issue", "create");

    let flush = run_obr(&workspace, ["sync", "--flush-only"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let jsonl_path = workspace.root.join(".obr").join("issues.jsonl");
    let issue_json = fs::read_to_string(&jsonl_path).expect("read issues jsonl");
    let mut phantom_issue: serde_json::Value =
        serde_json::from_str(issue_json.trim()).expect("parse current issue json");
    phantom_issue["id"] = serde_json::Value::String("second-9u2".to_string());
    phantom_issue["external_ref"] = serde_json::Value::Null;

    let base_snapshot_path = workspace.root.join(".obr").join("merge.base.jsonl");
    fs::write(
        &base_snapshot_path,
        format!(
            "{}\n",
            serde_json::to_string(&phantom_issue).expect("serialize phantom issue")
        ),
    )
    .expect("write base snapshot");

    let merge = run_obr(&workspace, ["sync", "--merge"], "merge");
    assert!(merge.status.success(), "merge failed: {}", merge.stderr);

    let show = run_obr(&workspace, ["show", &issue_id, "--json"], "show");
    assert!(show.status.success(), "show failed: {}", show.stderr);

    let final_issue_list: serde_json::Value =
        serde_json::from_str(&show.stdout).expect("parse final issue list");
    let final_issue = &final_issue_list[0];
    assert_eq!(final_issue["id"].as_str(), Some(issue_id.as_str()));

    let merged_jsonl = fs::read_to_string(&jsonl_path).expect("read merged jsonl");
    assert!(
        !merged_jsonl.contains("second-9u2"),
        "phantom base-only issue should not survive merge"
    );
}
