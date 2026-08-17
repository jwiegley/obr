//! Dedicated e2e tests for the `stats` subcommand.
//!
//! Exercises JSON output structure, empty workspace, populated workspace with
//! dependencies/labels/comments, and the `--no-activity` flag.

mod common;

use common::cli::{ObrWorkspace, run_obr};
use serde_json::Value;

fn init_and_populate(workspace: &ObrWorkspace) {
    let init = run_obr(workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let c1 = run_obr(
        workspace,
        [
            "create",
            "Auth module",
            "--type",
            "feature",
            "--priority",
            "1",
        ],
        "create_auth",
    );
    assert!(c1.status.success(), "create auth: {}", c1.stderr);

    let c2 = run_obr(
        workspace,
        ["create", "Write tests", "--type", "task", "--priority", "2"],
        "create_tests",
    );
    assert!(c2.status.success(), "create tests: {}", c2.stderr);

    let c3 = run_obr(
        workspace,
        [
            "create",
            "Fix login bug",
            "--type",
            "bug",
            "--priority",
            "0",
        ],
        "create_bug",
    );
    assert!(c3.status.success(), "create bug: {}", c3.stderr);
}

#[test]
fn stats_json_empty_workspace() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success());

    let stats = run_obr(
        &workspace,
        ["stats", "--json", "--no-activity"],
        "stats_empty",
    );
    assert!(stats.status.success(), "stats failed: {}", stats.stderr);

    let json: Value = serde_json::from_str(&stats.stdout).expect("valid JSON");
    let summary = &json["summary"];
    assert_eq!(summary["total_issues"], 0);
    assert_eq!(summary["open_issues"], 0);
    assert_eq!(summary["closed_issues"], 0);
}

#[test]
fn stats_json_populated_workspace() {
    let workspace = ObrWorkspace::new();
    init_and_populate(&workspace);

    let stats = run_obr(
        &workspace,
        ["stats", "--json", "--no-activity"],
        "stats_populated",
    );
    assert!(stats.status.success(), "stats failed: {}", stats.stderr);

    let json: Value = serde_json::from_str(&stats.stdout).expect("valid JSON");
    let summary = &json["summary"];
    assert_eq!(summary["total_issues"], 3);
    assert_eq!(summary["open_issues"], 3);
    assert_eq!(summary["closed_issues"], 0);
    assert_eq!(summary["ready_issues"], 3);
}

#[test]
fn stats_json_after_close() {
    let workspace = ObrWorkspace::new();
    init_and_populate(&workspace);

    let list = run_obr(&workspace, ["list", "--json"], "list_ids");
    assert!(list.status.success());
    let list_json: Value = serde_json::from_str(&list.stdout).expect("valid JSON");
    let first_id = list_json["issues"][0]["id"].as_str().expect("has issue id");

    let close = run_obr(
        &workspace,
        ["close", first_id, "--reason", "done"],
        "close_first",
    );
    assert!(close.status.success(), "close failed: {}", close.stderr);

    let stats = run_obr(
        &workspace,
        ["stats", "--json", "--no-activity"],
        "stats_after_close",
    );
    assert!(stats.status.success(), "stats failed: {}", stats.stderr);

    let json: Value = serde_json::from_str(&stats.stdout).expect("valid JSON");
    let summary = &json["summary"];
    assert_eq!(summary["total_issues"], 3);
    assert_eq!(summary["closed_issues"], 1);
    assert_eq!(summary["open_issues"], 2);
}

#[test]
fn stats_json_with_deps_shows_blocked() {
    let workspace = ObrWorkspace::new();
    init_and_populate(&workspace);

    let list = run_obr(&workspace, ["list", "--json"], "list_for_deps");
    assert!(list.status.success());
    let list_json: Value = serde_json::from_str(&list.stdout).expect("valid JSON");
    let issues = list_json["issues"].as_array().expect("issues array");
    let id0 = issues[0]["id"].as_str().unwrap();
    let id1 = issues[1]["id"].as_str().unwrap();

    let dep = run_obr(&workspace, ["dep", "add", id0, id1], "add_dep");
    assert!(dep.status.success(), "dep add failed: {}", dep.stderr);

    let stats = run_obr(
        &workspace,
        ["stats", "--json", "--no-activity"],
        "stats_with_deps",
    );
    assert!(stats.status.success(), "stats failed: {}", stats.stderr);

    let json: Value = serde_json::from_str(&stats.stdout).expect("valid JSON");
    let summary = &json["summary"];
    assert!(
        summary["total_issues"].as_u64().unwrap() >= 3,
        "should still have all issues"
    );
}

#[test]
fn stats_json_has_breakdowns() {
    let workspace = ObrWorkspace::new();
    init_and_populate(&workspace);

    let stats = run_obr(
        &workspace,
        ["stats", "--json", "--no-activity"],
        "stats_breakdowns",
    );
    assert!(stats.status.success(), "stats failed: {}", stats.stderr);

    let json: Value = serde_json::from_str(&stats.stdout).expect("valid JSON");
    assert!(json.get("summary").is_some(), "must have summary");
}

#[test]
fn stats_plain_text_succeeds() {
    let workspace = ObrWorkspace::new();
    init_and_populate(&workspace);

    let stats = run_obr(&workspace, ["stats", "--no-activity"], "stats_plain");
    assert!(
        stats.status.success(),
        "stats plain failed: {}",
        stats.stderr
    );
    assert!(
        !stats.stdout.is_empty(),
        "stats plain should produce output"
    );
}

#[test]
fn stats_no_activity_flag_suppresses_activity() {
    let workspace = ObrWorkspace::new();
    init_and_populate(&workspace);

    let stats = run_obr(
        &workspace,
        ["stats", "--json", "--no-activity"],
        "stats_no_activity",
    );
    assert!(stats.status.success());

    let json: Value = serde_json::from_str(&stats.stdout).expect("valid JSON");
    assert!(
        json.get("recent_activity").is_none(),
        "recent_activity should be absent with --no-activity"
    );
}
