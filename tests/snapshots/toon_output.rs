use super::common::cli::run_obr;
use super::init_workspace;
use insta::assert_snapshot;
use regex::Regex;
use std::fs;
use std::sync::LazyLock;

const TOON_JSONL_FIXTURE: &str = r#"{"id":"bd-blocker","title":"00 Blocking Root","description":"Unblocks dependent work","status":"open","priority":0,"issue_type":"task","created_at":"2026-02-01T00:00:00Z","created_by":"fixture","updated_at":"2026-02-01T00:00:00Z","source_repo":".","labels":["core"],"compaction_level":0,"original_size":0}
{"id":"bd-ready-p0","title":"01 Ready Critical Unassigned","status":"open","priority":0,"issue_type":"bug","created_at":"2026-02-02T00:00:00Z","created_by":"fixture","updated_at":"2026-02-02T00:00:00Z","source_repo":".","labels":["ops","agent"],"compaction_level":0,"original_size":0}
{"id":"bd-ready-p1-assigned","title":"02 Ready Assigned Feature","status":"open","priority":1,"issue_type":"feature","assignee":"alice","owner":"owner@example.com","created_at":"2026-02-03T00:00:00Z","created_by":"fixture","updated_at":"2026-02-03T00:00:00Z","source_repo":".","labels":["frontend"],"compaction_level":0,"original_size":0}
{"id":"bd-blocked","title":"03 Blocked By Root","status":"open","priority":1,"issue_type":"task","created_at":"2026-02-05T00:00:00Z","created_by":"fixture","updated_at":"2026-02-05T00:00:00Z","source_repo":".","labels":["blocked"],"dependencies":[{"issue_id":"bd-blocked","depends_on_id":"bd-blocker","type":"blocks","created_at":"2026-02-05T00:00:00Z","created_by":"fixture","metadata":"{}","thread_id":""}],"compaction_level":0,"original_size":0}
{"id":"bd-closed","title":"04 Closed Done","status":"closed","priority":2,"issue_type":"task","created_at":"2026-02-08T00:00:00Z","created_by":"fixture","updated_at":"2026-02-08T00:00:00Z","closed_at":"2026-02-08T01:00:00Z","close_reason":"done","source_repo":".","labels":["done"],"compaction_level":0,"original_size":0}
"#;

static TOON_GENERATED_AT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^generated_at:\s*.+$").expect("toon generated_at regex"));

fn init_toon_workspace() -> super::common::cli::ObrWorkspace {
    let workspace = init_workspace();
    super::common::cli::pin_jsonl(&workspace.root.join(".obr"));
    let jsonl_path = workspace.root.join(".obr/issues.jsonl");
    fs::write(jsonl_path, TOON_JSONL_FIXTURE).expect("write TOON JSONL fixture");

    let import = run_obr(
        &workspace,
        ["sync", "--import-only", "--json"],
        "toon_golden_import",
    );
    assert!(
        import.status.success(),
        "fixture import failed:\nstdout:\n{}\nstderr:\n{}",
        import.stdout,
        import.stderr
    );

    workspace
}

fn normalize_toon_output(raw: &str) -> String {
    let trimmed = raw.trim_end();
    TOON_GENERATED_AT_RE
        .replace_all(trimmed, "generated_at: GENERATED_AT")
        .to_string()
}

#[test]
fn toon_golden_list_output() {
    let workspace = init_toon_workspace();

    let output = run_obr(
        &workspace,
        ["list", "--all", "--sort", "title", "--format", "toon"],
        "toon_golden_list",
    );
    assert!(
        output.status.success(),
        "list --format toon failed: {}",
        output.stderr
    );
    assert!(
        !output.stdout.trim().is_empty(),
        "TOON output should not be empty"
    );

    let normalized = normalize_toon_output(&output.stdout);
    assert_snapshot!("toon_list_output", normalized);
}

#[test]
fn toon_golden_show_output() {
    let workspace = init_toon_workspace();

    let output = run_obr(
        &workspace,
        ["show", "bd-ready-p0", "--format", "toon"],
        "toon_golden_show",
    );
    assert!(
        output.status.success(),
        "show --format toon failed: {}",
        output.stderr
    );
    assert!(
        !output.stdout.trim().is_empty(),
        "TOON output should not be empty"
    );

    let normalized = normalize_toon_output(&output.stdout);
    assert_snapshot!("toon_show_output", normalized);
}

#[test]
fn toon_golden_ready_output() {
    let workspace = init_toon_workspace();

    let output = run_obr(
        &workspace,
        [
            "ready", "--sort", "priority", "--limit", "0", "--format", "toon",
        ],
        "toon_golden_ready",
    );
    assert!(
        output.status.success(),
        "ready --format toon failed: {}",
        output.stderr
    );
    assert!(
        !output.stdout.trim().is_empty(),
        "TOON output should not be empty"
    );

    let normalized = normalize_toon_output(&output.stdout);
    assert_snapshot!("toon_ready_output", normalized);
}
