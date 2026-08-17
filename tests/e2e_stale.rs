//! E2E tests for the `stale` command.

mod common;

use common::cli::{ObrWorkspace, extract_json_payload, parse_created_id, run_obr};
use serde_json::Value;

#[test]
fn e2e_stale_basic() {
    common::init_test_logging();
    let workspace = ObrWorkspace::new();
    run_obr(&workspace, ["init"], "init");

    // Create 3 issues
    run_obr(&workspace, ["create", "Issue 1"], "create1");
    run_obr(&workspace, ["create", "Issue 2"], "create2");
    run_obr(&workspace, ["create", "Issue 3"], "create3");

    // Check stale --days 0 (should include all, since updated_at <= now)
    // Actually, stale means updated BEFORE (now - days).
    // If days=0, threshold = now. updated_at <= now. So all issues are stale.
    let stale0 = run_obr(&workspace, ["stale", "--days", "0"], "stale0");
    assert!(stale0.status.success());
    // Should verify count
    let count = stale0.stdout.lines().filter(|l| l.contains(". [")).count();
    assert_eq!(count, 3, "All issues should be stale with days=0");

    // Check stale --days 1 (should include none, since updated_at > now - 1 day)
    let stale1 = run_obr(&workspace, ["stale", "--days", "1"], "stale1");
    assert!(stale1.status.success());
    let count1 = stale1.stdout.lines().filter(|l| l.contains(". [")).count();
    assert_eq!(
        count1, 0,
        "No issues should be stale with days=1 (freshly created)"
    );
}

#[test]
fn e2e_stale_json_output() {
    common::init_test_logging();
    let workspace = ObrWorkspace::new();
    run_obr(&workspace, ["init"], "init");
    run_obr(&workspace, ["create", "Issue JSON"], "create");

    let stale = run_obr(&workspace, ["stale", "--days", "0", "--json"], "stale_json");
    assert!(stale.status.success());

    let payload = extract_json_payload(&stale.stdout);
    let json: Vec<Value> = serde_json::from_str(&payload).expect("valid json");
    assert_eq!(json.len(), 1);
    assert_eq!(json[0]["title"], "Issue JSON");
    // Verify StaleIssue structure
    assert!(json[0].get("updated_at").is_some());
}

#[test]
fn e2e_stale_with_status_filter() {
    common::init_test_logging();
    let workspace = ObrWorkspace::new();
    run_obr(&workspace, ["init"], "init");

    run_obr(&workspace, ["create", "Open Issue"], "create1");

    let create2 = run_obr(&workspace, ["create", "InProgress Issue"], "create2");
    let id2 = parse_created_id(&create2.stdout);
    run_obr(
        &workspace,
        ["update", &id2, "--status", "in_progress"],
        "update2",
    );

    // Filter by status=open
    let stale_open = run_obr(
        &workspace,
        ["stale", "--days", "0", "--status", "open"],
        "stale_open",
    );
    let count_open = stale_open
        .stdout
        .lines()
        .filter(|l| l.contains("Open Issue"))
        .count();
    assert_eq!(count_open, 1);
    assert!(!stale_open.stdout.contains("InProgress Issue"));
}

#[test]
fn e2e_stale_with_deferred_status_filter() {
    common::init_test_logging();
    let workspace = ObrWorkspace::new();
    run_obr(&workspace, ["init"], "init");

    let open = run_obr(&workspace, ["create", "Open Issue"], "create_open");
    assert!(open.status.success(), "create open failed: {}", open.stderr);

    let deferred = run_obr(&workspace, ["create", "Deferred Issue"], "create_deferred");
    assert!(
        deferred.status.success(),
        "create deferred failed: {}",
        deferred.stderr
    );
    let deferred_id = parse_created_id(&deferred.stdout);
    let defer = run_obr(
        &workspace,
        [
            "update",
            &deferred_id,
            "--status",
            "deferred",
            "--defer",
            "2100-01-01T00:00:00Z",
        ],
        "defer_issue",
    );
    assert!(defer.status.success(), "defer failed: {}", defer.stderr);

    let stale = run_obr(
        &workspace,
        ["stale", "--days", "0", "--status", "deferred", "--json"],
        "stale_deferred",
    );
    assert!(
        stale.status.success(),
        "stale deferred failed: {}",
        stale.stderr
    );

    let payload = extract_json_payload(&stale.stdout);
    let json: Vec<Value> = serde_json::from_str(&payload).expect("valid json");
    assert_eq!(json.len(), 1);
    assert_eq!(json[0]["title"], "Deferred Issue");
    assert_eq!(json[0]["status"], "deferred");
}

#[test]
fn e2e_stale_default_includes_custom_nonterminal_statuses() {
    common::init_test_logging();
    let workspace = ObrWorkspace::new();
    run_obr(&workspace, ["init"], "init");

    let create = run_obr(&workspace, ["create", "Review Issue"], "create_review");
    assert!(create.status.success(), "create failed: {}", create.stderr);
    let issue_id = parse_created_id(&create.stdout);

    let update = run_obr(
        &workspace,
        ["update", &issue_id, "--status", "review"],
        "set_review_status",
    );
    assert!(update.status.success(), "update failed: {}", update.stderr);

    let stale = run_obr(
        &workspace,
        ["stale", "--days", "0", "--json"],
        "stale_review",
    );
    assert!(stale.status.success(), "stale failed: {}", stale.stderr);

    let payload = extract_json_payload(&stale.stdout);
    let json: Vec<Value> = serde_json::from_str(&payload).expect("valid json");
    assert!(
        json.iter()
            .any(|issue| issue["id"] == issue_id && issue["status"] == "review"),
        "custom nonterminal status should be included in default stale output"
    );
}
