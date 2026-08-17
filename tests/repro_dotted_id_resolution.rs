//! Regression for br-87u1b: exact dotted child IDs must continue resolving to
//! the child issue after rebuild/import cycles, even when the database also
//! contains tombstones.
//!
//! The failing installed `obr` binary in the swarm was returning an unrelated
//! tombstone for commands like `obr show br-8qdh0.11 --json` and rejecting
//! `obr update br-il53l.1 ...` as if the exact dotted ID were itself a
//! tombstone. Current `main` already contains the storage-side recovery path;
//! this test keeps that exact CLI contract covered.

mod common;

use common::cli::{ObrWorkspace, extract_json_payload, run_obr};
use serde_json::Value;

fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(&extract_json_payload(stdout)).expect("valid cli json payload")
}

fn first_issue(payload: &Value) -> &Value {
    payload
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(payload)
}

fn issue_id(payload: &Value) -> String {
    first_issue(payload)["id"]
        .as_str()
        .expect("issue id")
        .to_string()
}

#[test]
#[allow(clippy::too_many_lines)]
fn e2e_dotted_child_show_and_update_stay_on_exact_issue_after_rebuild() {
    let _log =
        common::test_log("e2e_dotted_child_show_and_update_stay_on_exact_issue_after_rebuild");
    let workspace = ObrWorkspace::new();

    let init = run_obr(
        &workspace,
        ["init", "--prefix", "dot"],
        "init_dotted_resolution",
    );
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let parent = run_obr(
        &workspace,
        ["create", "Parent issue", "--json"],
        "create_parent",
    );
    assert!(
        parent.status.success(),
        "parent create failed: {}",
        parent.stderr
    );
    let parent_id = issue_id(&parse_json(&parent.stdout));

    let child = run_obr(
        &workspace,
        ["create", "Child issue", "--parent", &parent_id, "--json"],
        "create_child",
    );
    assert!(
        child.status.success(),
        "child create failed: {}",
        child.stderr
    );
    let child_payload = parse_json(&child.stdout);
    let child_id = issue_id(&child_payload);
    assert!(
        child_id.starts_with(&format!("{parent_id}.")),
        "child id should be hierarchical, got {child_id} for parent {parent_id}"
    );

    let tombstone_seed = run_obr(
        &workspace,
        ["create", "Tombstone seed", "--json"],
        "create_tombstone_seed",
    );
    assert!(
        tombstone_seed.status.success(),
        "tombstone seed create failed: {}",
        tombstone_seed.stderr
    );
    let tombstone_id = issue_id(&parse_json(&tombstone_seed.stdout));

    let delete = run_obr(
        &workspace,
        [
            "delete",
            &tombstone_id,
            "--force",
            "--reason",
            "seed tombstone for dotted-id lookup regression",
        ],
        "delete_tombstone_seed",
    );
    assert!(
        delete.status.success(),
        "delete should create tombstone: stdout={} stderr={}",
        delete.stdout,
        delete.stderr
    );

    let flush = run_obr(&workspace, ["sync", "--flush-only"], "flush_before_rebuild");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let alt_db = workspace.root.join(".obr").join("beads.dotted-rebuilt.db");
    let alt_db_str = alt_db.to_string_lossy().to_string();
    let rebuild = run_obr(
        &workspace,
        [
            "--db",
            &alt_db_str,
            "sync",
            "--import-only",
            "--rebuild",
            "--json",
            "--no-auto-import",
            "--no-auto-flush",
        ],
        "rebuild_alt_db",
    );
    assert!(
        rebuild.status.success(),
        "rebuild failed: stdout={} stderr={}",
        rebuild.stdout,
        rebuild.stderr
    );

    let tombstone_show = run_obr(
        &workspace,
        [
            "--db",
            &alt_db_str,
            "--no-auto-import",
            "--no-auto-flush",
            "show",
            &tombstone_id,
            "--json",
        ],
        "show_tombstone_after_rebuild",
    );
    assert!(
        tombstone_show.status.success(),
        "show tombstone failed after rebuild: stdout={} stderr={}",
        tombstone_show.stdout,
        tombstone_show.stderr
    );
    let tombstone_payload = parse_json(&tombstone_show.stdout);
    assert_eq!(
        first_issue(&tombstone_payload)["id"].as_str(),
        Some(tombstone_id.as_str()),
        "tombstone lookup should still return the exact tombstone issue"
    );
    assert_eq!(
        first_issue(&tombstone_payload)["status"].as_str(),
        Some("tombstone"),
        "seed issue should remain a tombstone after rebuild"
    );

    for i in 0..10 {
        let show = run_obr(
            &workspace,
            [
                "--db",
                &alt_db_str,
                "--no-auto-import",
                "--no-auto-flush",
                "show",
                &child_id,
                "--json",
            ],
            &format!("show_child_{i}"),
        );
        assert!(
            show.status.success(),
            "show on dotted child failed after rebuild (loop {i}): stdout={} stderr={}",
            show.stdout,
            show.stderr
        );
        let shown = parse_json(&show.stdout);
        assert_eq!(
            first_issue(&shown)["id"].as_str(),
            Some(child_id.as_str()),
            "show should keep returning the exact dotted child after rebuild (loop {i})"
        );
        assert_eq!(
            first_issue(&shown)["parent"].as_str(),
            Some(parent_id.as_str()),
            "show should preserve the parent relationship for the dotted child (loop {i})"
        );
        assert_ne!(
            first_issue(&shown)["status"].as_str(),
            Some("tombstone"),
            "show should not drift to the tombstone seed issue (loop {i})"
        );

        let note = format!("touch {i}");
        let update = run_obr(
            &workspace,
            [
                "--db",
                &alt_db_str,
                "--no-auto-import",
                "--no-auto-flush",
                "update",
                &child_id,
                "--notes",
                &note,
                "--json",
            ],
            &format!("update_child_{i}"),
        );
        assert!(
            update.status.success(),
            "update on dotted child failed after rebuild (loop {i}): stdout={} stderr={}",
            update.stdout,
            update.stderr
        );
        assert!(
            !update.stderr.contains("cannot update tombstone issue"),
            "update misclassified the dotted child as a tombstone on loop {i}: {}",
            update.stderr
        );
        let updated = parse_json(&update.stdout);
        assert_eq!(
            first_issue(&updated)["id"].as_str(),
            Some(child_id.as_str()),
            "update should keep targeting the exact dotted child after rebuild (loop {i})"
        );
        assert_ne!(
            first_issue(&updated)["status"].as_str(),
            Some("tombstone"),
            "update should not surface the tombstone seed issue on loop {i}"
        );

        let show_after_update = run_obr(
            &workspace,
            [
                "--db",
                &alt_db_str,
                "--no-auto-import",
                "--no-auto-flush",
                "show",
                &child_id,
                "--json",
            ],
            &format!("show_child_after_update_{i}"),
        );
        assert!(
            show_after_update.status.success(),
            "show after dotted-child update failed on loop {i}: stdout={} stderr={}",
            show_after_update.stdout,
            show_after_update.stderr
        );
        let shown_after_update = parse_json(&show_after_update.stdout);
        assert_eq!(
            first_issue(&shown_after_update)["id"].as_str(),
            Some(child_id.as_str()),
            "show after update should still resolve the exact dotted child on loop {i}"
        );
        assert_ne!(
            first_issue(&shown_after_update)["status"].as_str(),
            Some("tombstone"),
            "show after update should not drift to the tombstone seed issue on loop {i}"
        );
    }
}
