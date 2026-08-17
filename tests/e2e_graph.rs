#![allow(clippy::similar_names)]

mod common;

use common::cli::{ObrWorkspace, extract_json_payload, run_obr, run_obr_with_env};
use serde_json::Value;
use toon_rust::try_decode;

fn parse_created_id(stdout: &str) -> String {
    let line = stdout.lines().next().unwrap_or("");
    // Handle both formats: "Created bd-xxx: title" and "✓ Created bd-xxx: title"
    let normalized = line.strip_prefix("✓ ").unwrap_or(line);
    let id_part = normalized
        .strip_prefix("Created ")
        .and_then(|rest| rest.split(':').next())
        .unwrap_or("");
    id_part.trim().to_string()
}

fn parse_json_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        let raw = value.to_string();
        raw.strip_suffix(".0")
            .unwrap_or(raw.as_str())
            .parse::<u64>()
            .ok()
    })
}

#[test]
fn e2e_graph_single_issue_no_dependents() {
    let _log = common::test_log("e2e_graph_single_issue_no_dependents");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let issue = run_obr(&workspace, ["create", "Standalone issue"], "create_issue");
    assert!(issue.status.success(), "create failed: {}", issue.stderr);
    let issue_id = parse_created_id(&issue.stdout);

    let graph = run_obr(&workspace, ["graph", &issue_id], "graph_single");
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);
    assert!(
        graph.stdout.contains("No dependents"),
        "Expected 'No dependents' message, got: {}",
        graph.stdout
    );
}

#[test]
fn e2e_graph_single_issue_with_dependents() {
    let _log = common::test_log("e2e_graph_single_issue_with_dependents");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Create blocking issue (root)
    let blocker = run_obr(&workspace, ["create", "Blocker issue"], "create_blocker");
    assert!(
        blocker.status.success(),
        "blocker create failed: {}",
        blocker.stderr
    );
    let blocker_id = parse_created_id(&blocker.stdout);

    // Create blocked issue (dependent)
    let blocked = run_obr(&workspace, ["create", "Blocked issue"], "create_blocked");
    assert!(
        blocked.status.success(),
        "blocked create failed: {}",
        blocked.stderr
    );
    let blocked_id = parse_created_id(&blocked.stdout);

    // Add dependency: blocked depends on blocker
    let dep_add = run_obr(
        &workspace,
        ["dep", "add", &blocked_id, &blocker_id],
        "dep_add",
    );
    assert!(
        dep_add.status.success(),
        "dep add failed: {}",
        dep_add.stderr
    );

    // Graph blocker - should show blocked as dependent
    let graph = run_obr(&workspace, ["graph", &blocker_id], "graph_blocker");
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);
    assert!(
        graph.stdout.contains("Dependents of"),
        "Expected 'Dependents of' message, got: {}",
        graph.stdout
    );
    assert!(
        graph.stdout.contains(&blocked_id),
        "Expected dependent issue ID in output, got: {}",
        graph.stdout
    );
}

#[test]
fn e2e_graph_single_issue_json() {
    let _log = common::test_log("e2e_graph_single_issue_json");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let blocker = run_obr(&workspace, ["create", "Blocker"], "create_blocker");
    assert!(
        blocker.status.success(),
        "blocker create failed: {}",
        blocker.stderr
    );
    let blocker_id = parse_created_id(&blocker.stdout);

    let blocked = run_obr(&workspace, ["create", "Blocked"], "create_blocked");
    assert!(
        blocked.status.success(),
        "blocked create failed: {}",
        blocked.stderr
    );
    let blocked_id = parse_created_id(&blocked.stdout);

    let dep_add = run_obr(
        &workspace,
        ["dep", "add", &blocked_id, &blocker_id],
        "dep_add",
    );
    assert!(
        dep_add.status.success(),
        "dep add failed: {}",
        dep_add.stderr
    );

    let graph = run_obr(&workspace, ["graph", &blocker_id, "--json"], "graph_json");
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);

    let payload = extract_json_payload(&graph.stdout);
    let json: Value = serde_json::from_str(&payload).expect("graph json");

    assert_eq!(json["root"], blocker_id, "root should be blocker id");
    assert_eq!(json["count"], 2, "count should be 2 (root + dependent)");

    let nodes = json["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 2, "should have 2 nodes");

    let edges = json["edges"].as_array().expect("edges array");
    assert_eq!(edges.len(), 1, "should have 1 edge");
    assert_eq!(edges[0][0], blocked_id, "edge from should be blocked");
    assert_eq!(edges[0][1], blocker_id, "edge to should be blocker");
}

#[test]
fn e2e_graph_single_issue_honors_toon_env_mode() {
    let _log = common::test_log("e2e_graph_single_issue_honors_toon_env_mode");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let blocker = run_obr(&workspace, ["create", "TOON blocker"], "create_blocker");
    assert!(
        blocker.status.success(),
        "blocker create failed: {}",
        blocker.stderr
    );
    let blocker_id = parse_created_id(&blocker.stdout);

    let blocked = run_obr(&workspace, ["create", "TOON blocked"], "create_blocked");
    assert!(
        blocked.status.success(),
        "blocked create failed: {}",
        blocked.stderr
    );
    let blocked_id = parse_created_id(&blocked.stdout);

    let dep_add = run_obr(
        &workspace,
        ["dep", "add", &blocked_id, &blocker_id],
        "dep_add",
    );
    assert!(
        dep_add.status.success(),
        "dep add failed: {}",
        dep_add.stderr
    );

    let graph = run_obr_with_env(
        &workspace,
        ["graph", &blocker_id],
        [("OBR_OUTPUT_FORMAT", "toon")],
        "graph_toon_env",
    );
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);

    let decoded = try_decode(graph.stdout.trim(), None).expect("valid graph TOON");
    let json = Value::from(decoded);
    assert_eq!(json["root"].as_str(), Some(blocker_id.as_str()));
    assert_eq!(parse_json_u64(&json["count"]), Some(2));
    assert_eq!(json["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(json["edges"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["edges"][0][0].as_str(), Some(blocked_id.as_str()));
    assert_eq!(json["edges"][0][1].as_str(), Some(blocker_id.as_str()));
}

#[test]
fn e2e_graph_single_issue_compact() {
    let _log = common::test_log("e2e_graph_single_issue_compact");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let blocker = run_obr(&workspace, ["create", "Blocker"], "create_blocker");
    assert!(
        blocker.status.success(),
        "blocker create failed: {}",
        blocker.stderr
    );
    let blocker_id = parse_created_id(&blocker.stdout);

    let blocked = run_obr(&workspace, ["create", "Blocked"], "create_blocked");
    assert!(
        blocked.status.success(),
        "blocked create failed: {}",
        blocked.stderr
    );
    let blocked_id = parse_created_id(&blocked.stdout);

    let dep_add = run_obr(
        &workspace,
        ["dep", "add", &blocked_id, &blocker_id],
        "dep_add",
    );
    assert!(
        dep_add.status.success(),
        "dep add failed: {}",
        dep_add.stderr
    );

    let graph = run_obr(
        &workspace,
        ["graph", &blocker_id, "--compact"],
        "graph_compact",
    );
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);

    // Compact format: root <- dependent
    assert!(
        graph.stdout.contains(&format!("{blocker_id} <-")),
        "Expected compact format with root, got: {}",
        graph.stdout
    );
    assert!(
        graph.stdout.contains(&blocked_id),
        "Expected dependent in compact output, got: {}",
        graph.stdout
    );
}

#[test]
fn e2e_graph_all_no_issues() {
    let _log = common::test_log("e2e_graph_all_no_issues");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let graph = run_obr(&workspace, ["graph", "--all"], "graph_all_empty");
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);
    assert!(
        graph.stdout.contains("No active issues found"),
        "Expected 'No issues' message, got: {}",
        graph.stdout
    );
}

#[test]
fn e2e_graph_all_includes_custom_status_issues() {
    let _log = common::test_log("e2e_graph_all_includes_custom_status_issues");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let review = run_obr(&workspace, ["create", "Review issue"], "create_review");
    assert!(
        review.status.success(),
        "create review failed: {}",
        review.stderr
    );
    let review_id = parse_created_id(&review.stdout);

    let update = run_obr(
        &workspace,
        ["update", &review_id, "--status", "review"],
        "set_review_status",
    );
    assert!(update.status.success(), "update failed: {}", update.stderr);

    let graph = run_obr(&workspace, ["graph", "--all", "--json"], "graph_all_review");
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);

    let payload = extract_json_payload(&graph.stdout);
    let json: Value = serde_json::from_str(&payload).expect("graph json");
    let components = json["components"].as_array().expect("components array");

    assert!(
        components.iter().any(|component| {
            component["nodes"]
                .as_array()
                .is_some_and(|nodes| nodes.iter().any(|node| node["id"] == review_id))
        }),
        "custom-status nonterminal issue should appear in graph --all output"
    );
}

#[test]
fn e2e_graph_all_with_connected_components() {
    let _log = common::test_log("e2e_graph_all_with_connected_components");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Create first connected component: A -> B
    let issue_a = run_obr(&workspace, ["create", "Issue A"], "create_a");
    assert!(
        issue_a.status.success(),
        "create a failed: {}",
        issue_a.stderr
    );
    let id_a = parse_created_id(&issue_a.stdout);

    let issue_b = run_obr(&workspace, ["create", "Issue B"], "create_b");
    assert!(
        issue_b.status.success(),
        "create b failed: {}",
        issue_b.stderr
    );
    let id_b = parse_created_id(&issue_b.stdout);

    let dep_ab = run_obr(&workspace, ["dep", "add", &id_b, &id_a], "dep_ab");
    assert!(dep_ab.status.success(), "dep add failed: {}", dep_ab.stderr);

    // Create second isolated issue (separate component)
    let issue_c = run_obr(&workspace, ["create", "Issue C"], "create_c");
    assert!(
        issue_c.status.success(),
        "create c failed: {}",
        issue_c.stderr
    );
    let id_c = parse_created_id(&issue_c.stdout);

    let graph = run_obr(&workspace, ["graph", "--all"], "graph_all");
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);

    // Should show both components
    assert!(
        graph.stdout.contains("2 component"),
        "Expected 2 components, got: {}",
        graph.stdout
    );
    assert!(
        graph.stdout.contains(&id_a) && graph.stdout.contains(&id_b),
        "Expected connected issues in output, got: {}",
        graph.stdout
    );
    assert!(
        graph.stdout.contains(&id_c),
        "Expected isolated issue in output, got: {}",
        graph.stdout
    );
}

#[test]
fn e2e_graph_all_json() {
    let _log = common::test_log("e2e_graph_all_json");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let issue_a = run_obr(&workspace, ["create", "Issue A"], "create_a");
    assert!(
        issue_a.status.success(),
        "create a failed: {}",
        issue_a.stderr
    );
    let id_a = parse_created_id(&issue_a.stdout);

    let issue_b = run_obr(&workspace, ["create", "Issue B"], "create_b");
    assert!(
        issue_b.status.success(),
        "create b failed: {}",
        issue_b.stderr
    );
    let id_b = parse_created_id(&issue_b.stdout);

    let dep_ab = run_obr(&workspace, ["dep", "add", &id_b, &id_a], "dep_ab");
    assert!(dep_ab.status.success(), "dep add failed: {}", dep_ab.stderr);

    let graph = run_obr(&workspace, ["graph", "--all", "--json"], "graph_all_json");
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);

    let payload = extract_json_payload(&graph.stdout);
    let json: Value = serde_json::from_str(&payload).expect("graph json");

    assert_eq!(json["total_nodes"], 2, "should have 2 total nodes");
    assert_eq!(
        json["total_components"], 1,
        "should have 1 component (connected)"
    );

    let components = json["components"].as_array().expect("components array");
    assert_eq!(components.len(), 1, "should have 1 component");

    let component = &components[0];
    let nodes = component["nodes"].as_array().expect("nodes array");
    assert_eq!(nodes.len(), 2, "component should have 2 nodes");

    let edges = component["edges"].as_array().expect("edges array");
    assert_eq!(edges.len(), 1, "component should have 1 edge");

    let roots = component["roots"].as_array().expect("roots array");
    assert_eq!(roots.len(), 1, "component should have 1 root");
    assert_eq!(roots[0], id_a, "root should be issue A");
}

#[test]
fn e2e_graph_requires_issue_or_all() {
    let _log = common::test_log("e2e_graph_requires_issue_or_all");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let graph = run_obr(&workspace, ["graph"], "graph_no_args");
    assert!(!graph.status.success(), "graph without args should fail");
    assert!(
        graph.stderr.contains("Issue ID required") || graph.stderr.contains("issue"),
        "Expected issue required error, got: {}",
        graph.stderr
    );
}

#[test]
fn e2e_graph_chain_depth() {
    let _log = common::test_log("e2e_graph_chain_depth");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Create chain: A -> B -> C (C depends on B, B depends on A)
    let issue_a = run_obr(&workspace, ["create", "Root issue A"], "create_a");
    assert!(
        issue_a.status.success(),
        "create a failed: {}",
        issue_a.stderr
    );
    let id_a = parse_created_id(&issue_a.stdout);

    let issue_b = run_obr(&workspace, ["create", "Middle issue B"], "create_b");
    assert!(
        issue_b.status.success(),
        "create b failed: {}",
        issue_b.stderr
    );
    let id_b = parse_created_id(&issue_b.stdout);

    let issue_c = run_obr(&workspace, ["create", "Leaf issue C"], "create_c");
    assert!(
        issue_c.status.success(),
        "create c failed: {}",
        issue_c.stderr
    );
    let id_c = parse_created_id(&issue_c.stdout);

    // B depends on A
    let dep_ba = run_obr(&workspace, ["dep", "add", &id_b, &id_a], "dep_ba");
    assert!(dep_ba.status.success(), "dep add failed: {}", dep_ba.stderr);

    // C depends on B
    let dep_cb = run_obr(&workspace, ["dep", "add", &id_c, &id_b], "dep_cb");
    assert!(dep_cb.status.success(), "dep add failed: {}", dep_cb.stderr);

    // Graph from A should show B at depth 1, C at depth 2
    let graph = run_obr(&workspace, ["graph", &id_a, "--json"], "graph_chain");
    assert!(graph.status.success(), "graph failed: {}", graph.stderr);

    let payload = extract_json_payload(&graph.stdout);
    let json: Value = serde_json::from_str(&payload).expect("graph json");

    assert_eq!(json["count"], 3, "should have 3 nodes");

    let nodes = json["nodes"].as_array().expect("nodes array");
    let node_a = nodes.iter().find(|n| n["id"] == id_a).expect("node A");
    let node_b = nodes.iter().find(|n| n["id"] == id_b).expect("node B");
    let node_c = nodes.iter().find(|n| n["id"] == id_c).expect("node C");

    assert_eq!(node_a["depth"], 0, "A should be at depth 0");
    assert_eq!(node_b["depth"], 1, "B should be at depth 1");
    assert_eq!(node_c["depth"], 2, "C should be at depth 2");
}
