//! E2E coverage for terminal-control sanitization in human output.

mod common;

use common::cli::{ObrWorkspace, parse_created_id, parse_json_value, run_obr};

fn assert_no_terminal_controls(output: &str) {
    for forbidden in ['\x1b', '\x07', '\x08', '\r', '\u{9b}'] {
        assert!(
            !output.contains(forbidden),
            "output contained raw control {forbidden:?}: {output:?}"
        );
    }
}

#[test]
fn human_output_escapes_terminal_controls_but_json_preserves_values() {
    let _log = common::test_log("human_output_escapes_terminal_controls_but_json_preserves_values");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let title = "screen\x1b[2J spoof\x08\r done";
    let create = run_obr(&workspace, ["create", title], "create_control_title");
    assert!(create.status.success(), "create failed: {}", create.stderr);
    assert_no_terminal_controls(&create.stdout);
    assert!(create.stdout.contains("\\u{1b}[2J"));
    assert!(create.stdout.contains("\\u{8}"));
    assert!(create.stdout.contains("\\r"));

    let id = parse_created_id(&create.stdout);
    assert!(!id.is_empty(), "missing created id: {}", create.stdout);

    let comment = "comment\x1b]52;c;bad\x07 tail";
    let author = "actor\x1b[31m";
    let add_comment = run_obr(
        &workspace,
        ["comments", "add", &id, "--author", author, comment],
        "add_control_comment",
    );
    assert!(
        add_comment.status.success(),
        "comment add failed: {}",
        add_comment.stderr
    );
    assert_no_terminal_controls(&add_comment.stdout);

    let show = run_obr(&workspace, ["show", &id], "show_human");
    assert!(show.status.success(), "show failed: {}", show.stderr);
    assert_no_terminal_controls(&show.stdout);
    assert!(show.stdout.contains("\\u{1b}[2J"));
    assert!(show.stdout.contains("\\u{1b}]52"));
    assert!(show.stdout.contains("\\u{7}"));

    let json = run_obr(&workspace, ["show", &id, "--json"], "show_json");
    assert!(json.status.success(), "json show failed: {}", json.stderr);
    let payload = parse_json_value(&json.stdout);
    let issue = payload
        .as_array()
        .and_then(|issues| issues.first())
        .expect("show --json should return one issue");
    assert_eq!(issue["title"].as_str(), Some(title));
    let comments = issue["comments"].as_array().expect("comments array");
    assert_eq!(comments[0]["author"].as_str(), Some(author));
    assert_eq!(comments[0]["text"].as_str(), Some(comment));
}
