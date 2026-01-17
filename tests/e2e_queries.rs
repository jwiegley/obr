mod common;

use common::cli::{BrWorkspace, extract_json_payload, run_br};
use serde_json::Value;

fn parse_created_id(stdout: &str) -> String {
    let line = stdout.lines().next().unwrap_or("");
    let id_part = line
        .strip_prefix("Created ")
        .and_then(|rest| rest.split(':').next())
        .unwrap_or("");
    id_part.trim().to_string()
}

#[test]
#[allow(clippy::similar_names, clippy::too_many_lines)]
fn e2e_queries_ready_stale_count_search() {
    let workspace = BrWorkspace::new();

    let init = run_br(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let blocker = run_br(
        &workspace,
        ["create", "Blocker issue", "-p", "1"],
        "create_blocker",
    );
    assert!(
        blocker.status.success(),
        "blocker create failed: {}",
        blocker.stderr
    );
    let blocker_id = parse_created_id(&blocker.stdout);

    let blocked = run_br(
        &workspace,
        ["create", "Blocked issue", "-p", "2"],
        "create_blocked",
    );
    assert!(
        blocked.status.success(),
        "blocked create failed: {}",
        blocked.stderr
    );
    let blocked_id = parse_created_id(&blocked.stdout);

    let deferred = run_br(
        &workspace,
        ["create", "Deferred issue", "-p", "3"],
        "create_deferred",
    );
    assert!(
        deferred.status.success(),
        "deferred create failed: {}",
        deferred.stderr
    );
    let deferred_id = parse_created_id(&deferred.stdout);

    let closed = run_br(
        &workspace,
        ["create", "Closed issue", "-p", "0"],
        "create_closed",
    );
    assert!(
        closed.status.success(),
        "closed create failed: {}",
        closed.stderr
    );
    let closed_id = parse_created_id(&closed.stdout);

    let label_blocker = run_br(
        &workspace,
        ["update", &blocker_id, "--add-label", "core"],
        "label_blocker",
    );
    assert!(
        label_blocker.status.success(),
        "label update failed: {}",
        label_blocker.stderr
    );

    let dep_add = run_br(
        &workspace,
        ["dep", "add", &blocked_id, &blocker_id],
        "dep_add",
    );
    assert!(
        dep_add.status.success(),
        "dep add failed: {}",
        dep_add.stderr
    );

    let defer_issue = run_br(
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
    assert!(
        defer_issue.status.success(),
        "defer update failed: {}",
        defer_issue.stderr
    );

    let close_issue = run_br(
        &workspace,
        ["update", &closed_id, "--status", "closed"],
        "close_issue",
    );
    assert!(
        close_issue.status.success(),
        "close update failed: {}",
        close_issue.stderr
    );

    let ready = run_br(&workspace, ["ready", "--json"], "ready");
    assert!(ready.status.success(), "ready failed: {}", ready.stderr);
    let ready_payload = extract_json_payload(&ready.stdout);
    let ready_json: Vec<Value> = serde_json::from_str(&ready_payload).expect("ready json");
    assert!(ready_json.iter().any(|item| item["id"] == blocker_id));
    assert!(!ready_json.iter().any(|item| item["id"] == blocked_id));
    assert!(!ready_json.iter().any(|item| item["id"] == deferred_id));

    let ready_text = run_br(&workspace, ["ready"], "ready_text");
    assert!(
        ready_text.status.success(),
        "ready text failed: {}",
        ready_text.stderr
    );
    assert!(
        ready_text.stdout.contains("Ready to work"),
        "ready text missing header"
    );

    let ready_core = run_br(
        &workspace,
        ["ready", "--json", "--label", "core"],
        "ready_label",
    );
    assert!(
        ready_core.status.success(),
        "ready label failed: {}",
        ready_core.stderr
    );
    let ready_core_payload = extract_json_payload(&ready_core.stdout);
    let ready_core_json: Vec<Value> =
        serde_json::from_str(&ready_core_payload).expect("ready label json");
    assert_eq!(ready_core_json.len(), 1);
    assert_eq!(ready_core_json[0]["id"], blocker_id);

    let blocked = run_br(&workspace, ["blocked", "--json"], "blocked");
    assert!(
        blocked.status.success(),
        "blocked failed: {}",
        blocked.stderr
    );
    let blocked_payload = extract_json_payload(&blocked.stdout);
    let blocked_json: Vec<Value> = serde_json::from_str(&blocked_payload).expect("blocked json");
    assert!(blocked_json.iter().any(|item| item["id"] == blocked_id));

    let blocked_text = run_br(&workspace, ["blocked"], "blocked_text");
    assert!(
        blocked_text.status.success(),
        "blocked text failed: {}",
        blocked_text.stderr
    );
    assert!(
        blocked_text.stdout.contains("Blocked Issues"),
        "blocked text missing header"
    );

    let search = run_br(
        &workspace,
        ["search", "Blocker", "--status", "open", "--json"],
        "search",
    );
    assert!(search.status.success(), "search failed: {}", search.stderr);
    let search_payload = extract_json_payload(&search.stdout);
    let search_json: Vec<Value> = serde_json::from_str(&search_payload).expect("search json");
    assert!(search_json.iter().any(|item| item["id"] == blocker_id));

    let search_text = run_br(&workspace, ["search", "Blocker"], "search_text");
    assert!(
        search_text.status.success(),
        "search text failed: {}",
        search_text.stderr
    );
    assert!(
        search_text.stdout.contains("Blocker issue"),
        "search text missing issue title"
    );

    let count = run_br(
        &workspace,
        ["count", "--by", "status", "--include-closed", "--json"],
        "count",
    );
    assert!(count.status.success(), "count failed: {}", count.stderr);
    let count_payload = extract_json_payload(&count.stdout);
    let count_json: Value = serde_json::from_str(&count_payload).expect("count json");
    assert_eq!(count_json["total"], 4);

    let groups = count_json["groups"].as_array().expect("count groups array");
    let mut counts = std::collections::BTreeMap::new();
    for group in groups {
        let key = group["group"].as_str().unwrap_or("").to_string();
        let value = group["count"].as_u64().unwrap_or(0);
        counts.insert(key, value);
    }
    assert_eq!(counts.get("open"), Some(&2));
    assert_eq!(counts.get("deferred"), Some(&1));
    assert_eq!(counts.get("closed"), Some(&1));

    let count_text = run_br(
        &workspace,
        ["count", "--by", "status", "--include-closed"],
        "count_text",
    );
    assert!(
        count_text.status.success(),
        "count text failed: {}",
        count_text.stderr
    );
    assert!(
        count_text.stdout.contains("Total:"),
        "count text missing total"
    );

    let count_priority = run_br(
        &workspace,
        [
            "count",
            "--by",
            "priority",
            "--priority",
            "0",
            "--include-closed",
            "--json",
        ],
        "count_priority",
    );
    assert!(
        count_priority.status.success(),
        "count priority failed: {}",
        count_priority.stderr
    );
    let count_priority_payload = extract_json_payload(&count_priority.stdout);
    let count_priority_json: Value =
        serde_json::from_str(&count_priority_payload).expect("count priority json");
    assert_eq!(count_priority_json["total"], 1);

    let stale = run_br(&workspace, ["stale", "--days", "0", "--json"], "stale");
    assert!(stale.status.success(), "stale failed: {}", stale.stderr);
    let stale_payload = extract_json_payload(&stale.stdout);
    let stale_json: Vec<Value> = serde_json::from_str(&stale_payload).expect("stale json");
    assert!(stale_json.len() >= 2);
    assert!(stale_json.iter().any(|item| item["id"] == blocker_id));
    assert!(stale_json.iter().any(|item| item["id"] == blocked_id));
}

/// E2E tests for stats command - text and JSON output.
#[test]
#[allow(clippy::too_many_lines)]
fn e2e_stats_command() {
    let workspace = BrWorkspace::new();

    let init = run_br(&workspace, ["init"], "stats_init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Create a few issues with different types and priorities
    let task1 = run_br(
        &workspace,
        ["create", "Task one", "-t", "task", "-p", "1"],
        "stats_create_task1",
    );
    assert!(task1.status.success(), "task1 failed: {}", task1.stderr);

    let bug1 = run_br(
        &workspace,
        ["create", "Bug one", "-t", "bug", "-p", "0"],
        "stats_create_bug1",
    );
    assert!(bug1.status.success(), "bug1 failed: {}", bug1.stderr);

    let feature1 = run_br(
        &workspace,
        ["create", "Feature one", "-t", "feature", "-p", "2"],
        "stats_create_feature1",
    );
    assert!(
        feature1.status.success(),
        "feature1 failed: {}",
        feature1.stderr
    );

    // Test stats text output
    let stats_text = run_br(&workspace, ["stats"], "stats_text");
    assert!(
        stats_text.status.success(),
        "stats text failed: {}",
        stats_text.stderr
    );
    assert!(
        stats_text.stdout.contains("Project Statistics"),
        "stats text missing header"
    );
    assert!(
        stats_text.stdout.contains("Total issues:"),
        "stats text missing total"
    );
    assert!(
        stats_text.stdout.contains("Open:"),
        "stats text missing open count"
    );

    // Test stats JSON output
    let stats_json = run_br(&workspace, ["stats", "--json"], "stats_json");
    assert!(
        stats_json.status.success(),
        "stats json failed: {}",
        stats_json.stderr
    );
    let stats_payload = extract_json_payload(&stats_json.stdout);
    let stats_parsed: Value = serde_json::from_str(&stats_payload).expect("stats json parse");
    assert!(stats_parsed["summary"]["total_issues"].as_u64().is_some());
    assert_eq!(stats_parsed["summary"]["total_issues"], 3);
    assert!(stats_parsed["summary"]["open_issues"].as_u64().is_some());

    // Test stats with --by-type
    let stats_by_type = run_br(&workspace, ["stats", "--by-type"], "stats_by_type");
    assert!(
        stats_by_type.status.success(),
        "stats by-type failed: {}",
        stats_by_type.stderr
    );
    assert!(
        stats_by_type.stdout.contains("By type:"),
        "stats by-type missing breakdown header"
    );
    assert!(
        stats_by_type.stdout.contains("task:") || stats_by_type.stdout.contains("task"),
        "stats by-type missing task type"
    );

    // Test stats with --by-priority
    let stats_by_priority = run_br(&workspace, ["stats", "--by-priority"], "stats_by_priority");
    assert!(
        stats_by_priority.status.success(),
        "stats by-priority failed: {}",
        stats_by_priority.stderr
    );
    assert!(
        stats_by_priority.stdout.contains("By priority:"),
        "stats by-priority missing breakdown header"
    );
    assert!(
        stats_by_priority.stdout.contains("P0:") || stats_by_priority.stdout.contains("P1:"),
        "stats by-priority missing priority levels"
    );

    // Test stats with multiple breakdowns
    let stats_combined = run_br(
        &workspace,
        ["stats", "--by-type", "--by-priority", "--json"],
        "stats_combined",
    );
    assert!(
        stats_combined.status.success(),
        "stats combined failed: {}",
        stats_combined.stderr
    );
    let combined_payload = extract_json_payload(&stats_combined.stdout);
    let combined_parsed: Value =
        serde_json::from_str(&combined_payload).expect("stats combined json parse");
    assert!(combined_parsed["summary"].is_object());

    // Check breakdowns array
    let breakdowns = combined_parsed["breakdowns"]
        .as_array()
        .expect("breakdowns array");
    assert!(!breakdowns.is_empty());

    // Verify specific breakdowns are present
    let has_type = breakdowns.iter().any(|b| b["dimension"] == "type");
    let has_priority = breakdowns.iter().any(|b| b["dimension"] == "priority");

    assert!(has_type, "missing type breakdown");
    assert!(has_priority, "missing priority breakdown");
}

/// E2E tests for config command - list, get, path.
#[test]
fn e2e_config_command() {
    let workspace = BrWorkspace::new();

    let init = run_br(&workspace, ["init"], "config_init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Test config --list
    let config_list = run_br(&workspace, ["config", "--list"], "config_list");
    assert!(
        config_list.status.success(),
        "config list failed: {}",
        config_list.stderr
    );
    assert!(
        config_list.stdout.contains("issue_prefix"),
        "config list missing issue_prefix"
    );
    assert!(
        config_list.stdout.contains("default_priority"),
        "config list missing default_priority"
    );
    assert!(
        config_list.stdout.contains("Default:"),
        "config list missing defaults"
    );

    // Test config --get for existing key
    let config_get = run_br(
        &workspace,
        ["config", "--get", "issue_prefix"],
        "config_get",
    );
    assert!(
        config_get.status.success(),
        "config get failed: {}",
        config_get.stderr
    );
    // The default prefix is 'bd'
    assert!(
        config_get.stdout.contains("bd"),
        "config get missing expected value"
    );

    // Test config --path
    let config_path = run_br(&workspace, ["config", "--path"], "config_path");
    assert!(
        config_path.status.success(),
        "config path failed: {}",
        config_path.stderr
    );
    assert!(
        config_path.stdout.contains("config.yaml")
            || config_path.stdout.contains("Config file paths"),
        "config path missing expected output"
    );

    // Test config --json output
    let config_json = run_br(&workspace, ["config", "--list", "--json"], "config_json");
    assert!(
        config_json.status.success(),
        "config json failed: {}",
        config_json.stderr
    );
    // Should output valid JSON
    let config_payload = extract_json_payload(&config_json.stdout);
    let _: Value = serde_json::from_str(&config_payload).expect("config json parse");
}

/// E2E tests for reopen command.
#[test]
fn e2e_reopen_command() {
    let workspace = BrWorkspace::new();

    let init = run_br(&workspace, ["init"], "reopen_init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Create an issue
    let create = run_br(
        &workspace,
        ["create", "Issue to reopen", "-p", "2"],
        "reopen_create",
    );
    assert!(create.status.success(), "create failed: {}", create.stderr);
    let issue_id = parse_created_id(&create.stdout);
    assert!(!issue_id.is_empty(), "failed to parse created ID");

    // Close the issue
    let close = run_br(
        &workspace,
        ["close", &issue_id, "--reason", "Testing reopen"],
        "reopen_close",
    );
    assert!(close.status.success(), "close failed: {}", close.stderr);

    // Verify it's closed
    let show_closed = run_br(
        &workspace,
        ["show", &issue_id, "--json"],
        "reopen_show_closed",
    );
    assert!(
        show_closed.status.success(),
        "show closed failed: {}",
        show_closed.stderr
    );
    let show_closed_payload = extract_json_payload(&show_closed.stdout);
    let show_closed_json: Value =
        serde_json::from_str(&show_closed_payload).expect("show closed json");

    // br show returns a list, so we access the first element
    if show_closed_json.is_array() {
        assert_eq!(show_closed_json[0]["status"], "closed");
    } else {
        // Fallback if behavior changes to return object for single ID
        assert_eq!(show_closed_json["status"], "closed");
    }

    // Reopen the issue
    let reopen = run_br(
        &workspace,
        ["reopen", &issue_id, "--reason", "Need more work"],
        "reopen_reopen",
    );
    assert!(reopen.status.success(), "reopen failed: {}", reopen.stderr);
    assert!(
        reopen.stdout.contains("Reopened") || reopen.stdout.contains(&issue_id),
        "reopen text missing confirmation"
    );

    // Verify it's open again
    let show_reopened = run_br(
        &workspace,
        ["show", &issue_id, "--json"],
        "reopen_show_reopened",
    );
    assert!(
        show_reopened.status.success(),
        "show reopened failed: {}",
        show_reopened.stderr
    );
    let show_reopened_payload = extract_json_payload(&show_reopened.stdout);
    let show_reopened_json: Value =
        serde_json::from_str(&show_reopened_payload).expect("show reopened json");

    if show_reopened_json.is_array() {
        assert_eq!(show_reopened_json[0]["status"], "open");
    } else {
        assert_eq!(show_reopened_json["status"], "open");
    }

    // Test reopen with JSON output
    let close_again = run_br(&workspace, ["close", &issue_id], "reopen_close_again");
    assert!(
        close_again.status.success(),
        "close again failed: {}",
        close_again.stderr
    );

    let reopen_json = run_br(
        &workspace,
        ["reopen", &issue_id, "--json"],
        "reopen_reopen_json",
    );
    assert!(
        reopen_json.status.success(),
        "reopen json failed: {}",
        reopen_json.stderr
    );
    let reopen_payload = extract_json_payload(&reopen_json.stdout);
    let reopen_parsed: Value = serde_json::from_str(&reopen_payload).expect("reopen json parse");

    // Check reopened array
    let reopened = reopen_parsed["reopened"]
        .as_array()
        .expect("reopened array");
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0]["id"], issue_id);
    assert_eq!(reopened[0]["status"], "open");
}
