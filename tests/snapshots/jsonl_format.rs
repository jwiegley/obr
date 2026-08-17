use super::common::cli::run_obr;
use super::{create_issue, init_workspace, normalize_jsonl};
use insta::assert_snapshot;
use std::fs;

#[test]
fn snapshot_jsonl_export() {
    let workspace = init_workspace();
    super::common::cli::pin_jsonl(&workspace.root.join(".obr"));
    create_issue(&workspace, "JSONL issue one", "create_jsonl_one");
    create_issue(&workspace, "JSONL issue two", "create_jsonl_two");

    let sync = run_obr(&workspace, ["sync", "--flush-only"], "sync_flush");
    assert!(sync.status.success(), "sync failed: {}", sync.stderr);

    let jsonl_path = workspace.root.join(".obr").join("issues.jsonl");
    let contents = fs::read_to_string(&jsonl_path).expect("read jsonl");
    let normalized = normalize_jsonl(&contents);

    assert_snapshot!("issues_jsonl_export", normalized);
}
