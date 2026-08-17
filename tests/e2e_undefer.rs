use crate::common::cli::{ObrWorkspace, run_obr};
mod common;

#[test]
fn test_soft_defer_behavior() {
    let workspace = ObrWorkspace::new();
    run_obr(&workspace, ["init"], "init");

    // Create issue
    let create = run_obr(&workspace, ["create", "Soft Defer"], "create");
    // Extract ID (e.g. "✓ Created bd-1: Soft Defer" - ID is the 3rd word)
    let id_line = create.stdout.lines().next().unwrap();
    let id = id_line
        .split_whitespace()
        .nth(2)
        .unwrap()
        .trim_end_matches(':');

    // Soft defer using update (sets date but not status)
    // Note: status remains 'open' by default if not specified
    let update = run_obr(&workspace, ["update", id, "--defer", "+1d"], "update");
    assert!(update.status.success());

    // Check status is still open?
    // Use JSON for reliable checking (text output format varies)
    let show = run_obr(&workspace, ["show", id, "--json"], "show");
    let json: serde_json::Value = serde_json::from_str(&show.stdout).expect("parse json");
    // show returns list of details
    let issue_details = if json.is_array() { &json[0] } else { &json };
    // Issue details are flattened, so status is at the top level
    let status = issue_details["status"].as_str().expect("status string");
    assert_eq!(status, "open", "Status should remain open");

    // Check ready - should be excluded
    let ready = run_obr(&workspace, ["ready"], "ready");
    assert!(
        !ready.stdout.contains(id),
        "Soft deferred issue should be excluded from ready"
    );

    // Try undefer
    let undefer = run_obr(&workspace, ["undefer", id], "undefer");
    // If bug exists, this will say skipped/not deferred
    println!("Undefer output: {}", undefer.stdout);

    // Expectation: user should be able to undefer any deferred issue (date or status)
    // or update should have set status.
    // If undefer fails to clear date, it's a bug.
    // Let's verify if date was cleared.

    let _show_after = run_obr(&workspace, ["show", id], "show_after");
    // If cleared, it won't show "until ..." (unless show logic hides it? show displays description etc)
    // show doesn't explicitly list defer_until in standard output?
    // It does! "  [open] (until ...)" ? No, wait.
    // src/cli/commands/show.rs doesn't print defer_until in standard output?
    // Let's check show.rs.
    // It prints ID, Title, Status. Labels. Description. Deps. Comments.
    // It DOES NOT print defer_until!
    // So checking "show" text output is insufficient.
    // Need --json.

    let show_json = run_obr(&workspace, ["show", id, "--json"], "show_json");
    // Parse JSON
    let json: serde_json::Value = serde_json::from_str(&show_json.stdout).unwrap();
    // The issue might be in an array (show returns list)
    let issue = if json.is_array() { &json[0] } else { &json };

    // field should be missing or null
    assert!(
        issue.get("defer_until").is_none() || issue["defer_until"].is_null(),
        "defer_until should be null/missing, got: {:?}",
        issue.get("defer_until")
    );
}

#[test]
fn test_soft_defer_preserves_in_progress_status() {
    let workspace = ObrWorkspace::new();
    run_obr(&workspace, ["init"], "init");

    let create = run_obr(
        &workspace,
        ["create", "Soft Defer In Progress", "--json"],
        "create",
    );
    assert!(create.status.success());
    let created: serde_json::Value = serde_json::from_str(&create.stdout).expect("parse json");
    let id = created["id"].as_str().expect("issue id");

    let update = run_obr(
        &workspace,
        ["update", id, "--status", "in_progress", "--defer", "+1d"],
        "update_soft_defer_in_progress",
    );
    assert!(update.status.success(), "update failed: {}", update.stderr);

    let undefer = run_obr(&workspace, ["undefer", id, "--json"], "undefer");
    assert!(
        undefer.status.success(),
        "undefer failed: {}",
        undefer.stderr
    );

    let show_json = run_obr(&workspace, ["show", id, "--json"], "show_json");
    let json: serde_json::Value = serde_json::from_str(&show_json.stdout).expect("parse json");
    let issue = if json.is_array() { &json[0] } else { &json };

    assert_eq!(
        issue["status"].as_str(),
        Some("in_progress"),
        "soft-deferred in-progress issue should stay in progress"
    );
    assert!(
        issue.get("defer_until").is_none() || issue["defer_until"].is_null(),
        "defer_until should be cleared, got: {:?}",
        issue.get("defer_until")
    );
}
