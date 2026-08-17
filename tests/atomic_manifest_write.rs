//! Regression tests for atomic manifest writing (beads_rust-jnxv).
//!
//! Verifies that `.manifest.json` is written atomically via temp file +
//! fsync + durable_rename, so a crash or interruption never leaves a
//! torn/corrupt manifest.

mod common;

use common::cli::{ObrWorkspace, run_obr};
use std::fs;

fn init_and_create_issue(workspace: &ObrWorkspace) {
    let init = run_obr(workspace, ["init", "--prefix", "bd"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let create = run_obr(
        workspace,
        ["create", "--title", "test issue", "--no-auto-flush"],
        "create",
    );
    assert!(create.status.success(), "create failed: {}", create.stderr);
}

#[test]
fn manifest_is_valid_json_after_flush() {
    let workspace = ObrWorkspace::new();
    init_and_create_issue(&workspace);

    let flush = run_obr(&workspace, ["sync", "--flush-only", "--manifest"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let manifest_path = workspace.root.join(".obr").join(".manifest.json");
    assert!(
        manifest_path.exists(),
        "manifest should exist after --manifest flush"
    );

    let content = fs::read_to_string(&manifest_path).expect("read manifest");
    let parsed: serde_json::Value =
        serde_json::from_str(&content).expect("manifest should be valid JSON");
    assert!(parsed.is_object(), "manifest should be a JSON object");
    assert!(
        parsed.get("export_time").is_some(),
        "manifest should have export_time"
    );
    assert!(
        parsed.get("issues_count").is_some(),
        "manifest should have issues_count"
    );
    assert!(
        parsed.get("content_hash").is_some(),
        "manifest should have content_hash"
    );
}

#[test]
fn manifest_write_leaves_no_temp_files() {
    let workspace = ObrWorkspace::new();
    init_and_create_issue(&workspace);

    let flush = run_obr(&workspace, ["sync", "--flush-only", "--manifest"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let obr_dir = workspace.root.join(".obr");
    let temp_files: Vec<_> = fs::read_dir(&obr_dir)
        .expect("read .obr dir")
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();

    assert!(
        temp_files.is_empty(),
        "no .tmp files should remain after successful flush, found: {:?}",
        temp_files
            .iter()
            .map(std::fs::DirEntry::file_name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn pre_existing_manifest_survives_if_no_manifest_flag() {
    let workspace = ObrWorkspace::new();
    init_and_create_issue(&workspace);

    let manifest_path = workspace.root.join(".obr").join(".manifest.json");
    let sentinel = r#"{"sentinel": true}"#;
    fs::write(&manifest_path, sentinel).expect("write sentinel manifest");

    let flush = run_obr(&workspace, ["sync", "--flush-only"], "flush-no-manifest");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let content = fs::read_to_string(&manifest_path).expect("read manifest");
    assert_eq!(
        content, sentinel,
        "manifest should be untouched when --manifest is not passed"
    );
}

#[test]
fn manifest_overwrite_replaces_old_content_atomically() {
    let workspace = ObrWorkspace::new();
    init_and_create_issue(&workspace);

    let manifest_path = workspace.root.join(".obr").join(".manifest.json");
    let old_content = r#"{"old": "manifest", "issues_count": 0}"#;
    fs::write(&manifest_path, old_content).expect("write old manifest");

    let flush = run_obr(
        &workspace,
        ["sync", "--flush-only", "--manifest"],
        "flush-overwrite",
    );
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let new_content = fs::read_to_string(&manifest_path).expect("read manifest");
    assert_ne!(
        new_content, old_content,
        "manifest should be updated after flush"
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&new_content).expect("new manifest should be valid JSON");
    assert!(
        parsed.get("issues_count").is_some(),
        "new manifest should have issues_count"
    );
    let count = parsed["issues_count"]
        .as_u64()
        .expect("issues_count should be u64");
    assert!(
        count >= 1,
        "should have exported at least 1 issue, got {count}"
    );
}
