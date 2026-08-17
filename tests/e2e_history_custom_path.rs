mod common;

use common::cli::{ObrWorkspace, run_obr, run_obr_with_env};

/// Helper to create an issue without auto-flush.
fn create_issue(workspace: &ObrWorkspace, title: &str, label: &str) {
    let create = run_obr(workspace, ["--no-auto-flush", "create", title], label);
    assert!(create.status.success(), "create failed: {}", create.stderr);
}

#[test]
fn e2e_history_custom_path() {
    let _log = common::test_log("e2e_history_custom_path");
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Create an issue so we have something to sync
    create_issue(&workspace, "Issue 1", "create1");

    // 1. Initial sync to create the file
    let sync1 = run_obr(&workspace, ["sync", "--flush-only"], "sync1");
    assert!(sync1.status.success());

    // 2. Create another issue to trigger a change
    create_issue(&workspace, "Issue 2", "create2");

    // 3. Sync with CUSTOM path via ENV VAR (First time)
    // This creates .obr/custom.jsonl. No backup yet because it didn't exist.
    let sync2 = run_obr_with_env(
        &workspace,
        ["sync", "--flush-only", "--allow-external-jsonl"],
        vec![("OBR_JSONL", ".obr/custom.jsonl")],
        "sync2",
    );
    assert!(sync2.status.success(), "sync2 failed: {}", sync2.stderr);

    // 4. Create another issue to trigger change
    create_issue(&workspace, "Issue 3", "create3");

    // 5. Sync with CUSTOM path again (Second time)
    // This overwrites .obr/custom.jsonl. Backup SHOULD be created now.
    let sync3 = run_obr_with_env(
        &workspace,
        ["sync", "--flush-only", "--allow-external-jsonl"],
        vec![("OBR_JSONL", ".obr/custom.jsonl")],
        "sync3",
    );
    assert!(sync3.status.success(), "sync3 failed: {}", sync3.stderr);

    // 6. Check history
    let list = run_obr(&workspace, ["history", "list"], "history_list");

    // NEW BEHAVIOR (FIXED): Backup found for custom file
    assert!(
        list.stdout.contains("custom."),
        "Failure: Backup NOT created for custom file. Output:\n{}",
        list.stdout
    );
}
