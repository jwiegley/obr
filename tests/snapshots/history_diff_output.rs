use super::common::cli::{ObrWorkspace, run_obr};
use super::init_workspace;
use insta::assert_snapshot;
use serde_json::Value;
use std::fs;

// Versioned history-diff fixtures. Keep these hand-written and compact so
// review diffs show exactly which JSONL lines changed.
//
// Golden update workflow:
// INSTA_UPDATE=always rch exec -- cargo test --test snapshots history_diff_golden
//
// Review the text diff and JSON summary snapshots together. The text diff is
// the detailed human artifact; the JSON summary is the concise machine surface.
const HISTORY_BACKUP_NAME: &str = "issues.v0.1.0.20260401_000000_000000.jsonl";
const HISTORY_BACKUP_JSONL: &str = r#"{"id":"bd-history-alpha","title":"Implement sync planner","description":"Initial planner accepts JSONL exports","status":"open","priority":2,"issue_type":"task","created_at":"2026-04-01T00:00:00Z","created_by":"fixture","updated_at":"2026-04-01T00:00:00Z","source_repo":".","labels":["sync"],"compaction_level":0,"original_size":0}
{"id":"bd-history-beta","title":"Fix CLI rendering","description":"Plain output drops one column","status":"open","priority":1,"issue_type":"bug","created_at":"2026-04-01T00:10:00Z","created_by":"fixture","updated_at":"2026-04-01T00:10:00Z","source_repo":".","labels":["cli"],"compaction_level":0,"original_size":0}
{"id":"bd-history-gamma","title":"Wire dependency graph","description":"Graph waits for the sync planner","status":"open","priority":2,"issue_type":"task","created_at":"2026-04-01T00:20:00Z","created_by":"fixture","updated_at":"2026-04-01T00:20:00Z","source_repo":".","labels":["graph"],"dependencies":[{"issue_id":"bd-history-gamma","depends_on_id":"bd-history-alpha","type":"blocks","created_at":"2026-04-01T00:20:00Z","created_by":"fixture","metadata":"{}","thread_id":""}],"compaction_level":0,"original_size":0}
{"id":"bd-history-notes","title":"Document history command","description":"Initial release notes","status":"open","priority":3,"issue_type":"docs","created_at":"2026-04-01T00:30:00Z","created_by":"fixture","updated_at":"2026-04-01T00:30:00Z","source_repo":".","labels":["docs"],"comments":[{"id":1,"issue_id":"bd-history-notes","author":"fixture","text":"Draft the operator-facing example.","created_at":"2026-04-01T00:35:00Z"}],"compaction_level":0,"original_size":0}
{"id":"bd-history-removed","title":"Remove legacy export","description":"Old exporter should be deleted","status":"open","priority":3,"issue_type":"task","created_at":"2026-04-01T00:40:00Z","created_by":"fixture","updated_at":"2026-04-01T00:40:00Z","source_repo":".","labels":["cleanup"],"compaction_level":0,"original_size":0}
"#;
const HISTORY_CURRENT_JSONL: &str = r#"{"id":"bd-history-alpha","title":"Implement resilient sync planner","description":"Planner accepts JSONL exports and rejects invalid merges","status":"closed","priority":1,"issue_type":"feature","created_at":"2026-04-01T00:00:00Z","created_by":"fixture","updated_at":"2026-04-02T00:00:00Z","closed_at":"2026-04-02T00:00:00Z","close_reason":"released in v0.1.1","source_repo":".","labels":["sync","release"],"compaction_level":0,"original_size":0}
{"id":"bd-history-beta","title":"Fix CLI rendering","description":"Plain output includes all expected columns","status":"open","priority":0,"issue_type":"bug","created_at":"2026-04-01T00:10:00Z","created_by":"fixture","updated_at":"2026-04-02T00:10:00Z","source_repo":".","labels":["cli","regression"],"compaction_level":0,"original_size":0}
{"id":"bd-history-gamma","title":"Wire dependency graph","description":"Graph now waits for rendering and sync planner","status":"open","priority":2,"issue_type":"task","created_at":"2026-04-01T00:20:00Z","created_by":"fixture","updated_at":"2026-04-02T00:20:00Z","source_repo":".","labels":["graph"],"dependencies":[{"issue_id":"bd-history-gamma","depends_on_id":"bd-history-alpha","type":"blocks","created_at":"2026-04-01T00:20:00Z","created_by":"fixture","metadata":"{}","thread_id":""},{"issue_id":"bd-history-gamma","depends_on_id":"bd-history-beta","type":"related","created_at":"2026-04-02T00:20:00Z","created_by":"fixture","metadata":"{}","thread_id":""}],"compaction_level":0,"original_size":0}
{"id":"bd-history-notes","title":"Document history diff command","description":"Release notes include diff and restore examples","status":"open","priority":2,"issue_type":"docs","created_at":"2026-04-01T00:30:00Z","created_by":"fixture","updated_at":"2026-04-02T00:30:00Z","source_repo":".","labels":["docs"],"comments":[{"id":1,"issue_id":"bd-history-notes","author":"fixture","text":"Draft the operator-facing example.","created_at":"2026-04-01T00:35:00Z"},{"id":2,"issue_id":"bd-history-notes","author":"reviewer","text":"Add the v0.1.0 to v0.1.1 sample diff.","created_at":"2026-04-02T00:35:00Z"}],"compaction_level":0,"original_size":0}
{"id":"bd-history-removed","title":"Remove legacy export","description":"Old exporter should be deleted","status":"tombstone","priority":3,"issue_type":"task","created_at":"2026-04-01T00:40:00Z","created_by":"fixture","updated_at":"2026-04-02T00:40:00Z","deleted_at":"2026-04-02T00:40:00Z","deleted_by":"fixture","delete_reason":"replaced by sync planner","original_type":"task","source_repo":".","labels":["cleanup"],"compaction_level":0,"original_size":0}
{"id":"bd-history-new","title":"Add merge audit output","description":"New issue introduced after v0.1.0","status":"open","priority":1,"issue_type":"task","created_at":"2026-04-02T01:00:00Z","created_by":"fixture","updated_at":"2026-04-02T01:00:00Z","source_repo":".","labels":["audit"],"compaction_level":0,"original_size":0}
"#;

fn init_history_diff_workspace() -> ObrWorkspace {
    let workspace = init_workspace();
    super::common::cli::pin_jsonl(&workspace.root.join(".obr"));
    let obr_dir = workspace.root.join(".obr");
    let history_dir = obr_dir.join("history");
    fs::create_dir_all(&history_dir).expect("create history directory");

    fs::write(obr_dir.join("issues.jsonl"), HISTORY_CURRENT_JSONL)
        .expect("write current version JSONL");
    let backup_path = history_dir.join(HISTORY_BACKUP_NAME);
    fs::write(&backup_path, HISTORY_BACKUP_JSONL).expect("write backup version JSONL");
    fs::write(
        backup_path.with_extension("jsonl.meta.json"),
        r#"{"target":{"kind":"relative","path":"issues.jsonl"}}"#,
    )
    .expect("write backup target metadata");

    workspace
}

fn normalize_history_output(raw: &str, workspace: &ObrWorkspace) -> String {
    // `obr` canonicalizes the workspace path, which on macOS resolves
    // /var/folders/... to /private/var/folders/.... Substituting only the
    // uncanonicalized root would leave a platform-specific `/private` prefix
    // baked into the snapshot, so replace the canonical form first.
    let mut normalized = raw.trim_end().replace('\\', "/");
    let canonical = dunce::canonicalize(&workspace.root).unwrap_or_else(|_| workspace.root.clone());
    for root in [canonical, workspace.root.clone()] {
        normalized = normalized.replace(&root.to_string_lossy().replace('\\', "/"), "$WORKSPACE");
    }
    normalized
}

fn assert_valid_json(raw: &str, context: &str) {
    let error = serde_json::from_str::<Value>(raw)
        .err()
        .map(|err| format!("{context} did not emit valid JSON: {err}\n\n{raw}"));
    assert_eq!(None, error);
}

#[test]
fn history_diff_golden_text_v0_1_0_to_v0_1_1() {
    let workspace = init_history_diff_workspace();

    let output = run_obr(
        &workspace,
        ["history", "diff", HISTORY_BACKUP_NAME],
        "history_diff_text_golden",
    );
    assert!(
        output.status.success(),
        "history diff failed: {}",
        output.stderr
    );

    assert_snapshot!(
        "history_diff_text_v0_1_0_to_v0_1_1",
        normalize_history_output(&output.stdout, &workspace)
    );
}

#[test]
fn history_diff_golden_json_summary_v0_1_0_to_v0_1_1() {
    let workspace = init_history_diff_workspace();

    let output = run_obr(
        &workspace,
        ["--json", "history", "diff", HISTORY_BACKUP_NAME],
        "history_diff_json_golden",
    );
    assert!(
        output.status.success(),
        "history diff --json failed: {}",
        output.stderr
    );

    let normalized = normalize_history_output(&output.stdout, &workspace);
    assert_valid_json(&normalized, "history diff --json");
    assert_snapshot!("history_diff_json_summary_v0_1_0_to_v0_1_1", normalized);
}
