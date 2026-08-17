use super::common::cli::{ObrWorkspace, run_obr};
use super::{create_issue, init_workspace, normalize_output};
use insta::assert_snapshot;

/// `obr --help` lists `serve` only when the optional `mcp` feature is compiled
/// in, so the top-level command list is feature-set dependent. CI runs
/// `cargo test --all-features` (`.github/workflows/ci.yml`) while the default
/// developer build leaves `mcp` off, so freezing a single golden for both makes
/// whichever build was not captured fail. Snapshot each feature set under its
/// own name.
#[cfg(feature = "mcp")]
#[test]
fn snapshot_help_output() {
    let workspace = ObrWorkspace::new();
    let output = run_obr(&workspace, ["--help"], "help");
    assert!(output.status.success(), "help failed: {}", output.stderr);
    assert!(
        output.stdout.contains("serve"),
        "help should list the serve subcommand with the mcp feature"
    );
    assert_snapshot!("help_output", normalize_output(&output.stdout));
}

#[cfg(not(feature = "mcp"))]
#[test]
fn snapshot_help_output_no_mcp() {
    let workspace = ObrWorkspace::new();
    let output = run_obr(&workspace, ["--help"], "help");
    assert!(output.status.success(), "help failed: {}", output.stderr);
    assert!(
        !output.stdout.contains("serve"),
        "help should not list the serve subcommand without the mcp feature"
    );
    assert_snapshot!("help_output_no_mcp", normalize_output(&output.stdout));
}

/// The self-updater was removed outright (there is no release feed for this
/// fork), so `upgrade` must not reappear in help under any feature set.
#[test]
fn snapshot_help_output_no_upgrade() {
    let workspace = ObrWorkspace::new();
    let output = run_obr(&workspace, ["--help"], "help");
    assert!(output.status.success(), "help failed: {}", output.stderr);
    let stdout = &output.stdout;
    assert!(
        !stdout.contains("upgrade"),
        "help must not list an upgrade subcommand: the self-updater was removed"
    );
    for cmd in ["create", "list", "show", "close", "search"] {
        assert!(
            stdout.contains(cmd),
            "help should list core subcommand '{cmd}'"
        );
    }
}

#[test]
fn snapshot_create_help() {
    let workspace = ObrWorkspace::new();
    let output = run_obr(&workspace, ["create", "--help"], "create_help");
    assert!(
        output.status.success(),
        "create help failed: {}",
        output.stderr
    );
    assert_snapshot!("create_help", normalize_output(&output.stdout));
}

#[test]
fn snapshot_list_empty() {
    let workspace = init_workspace();
    let output = run_obr(&workspace, ["list"], "list_empty");
    assert!(output.status.success(), "list failed: {}", output.stderr);
    assert_snapshot!("list_empty", normalize_output(&output.stdout));
}

#[test]
fn snapshot_list_with_issues() {
    let workspace = init_workspace();
    create_issue(&workspace, "Bug: Fix login", "create_bug");
    create_issue(&workspace, "Feature: Add dark mode", "create_feature");
    create_issue(&workspace, "Task: Update docs", "create_task");

    let output = run_obr(&workspace, ["list"], "list_with_issues");
    assert!(output.status.success(), "list failed: {}", output.stderr);
    assert_snapshot!("list_with_issues", normalize_output(&output.stdout));
}

#[test]
fn snapshot_show_output() {
    let workspace = init_workspace();
    let id = create_issue(&workspace, "Test issue with description", "create_show");

    let output = run_obr(&workspace, ["show", &id], "show_text");
    assert!(output.status.success(), "show failed: {}", output.stderr);
    assert_snapshot!("show_output", normalize_output(&output.stdout));
}

#[test]
fn snapshot_ready_output() {
    let workspace = init_workspace();
    // Create issues with different priorities using update
    let id1 = create_issue(&workspace, "Critical bug", "create_p0");
    let id2 = create_issue(&workspace, "High priority feature", "create_p1");
    let id3 = create_issue(&workspace, "Medium task", "create_p2");

    // Update priorities
    let _ = run_obr(&workspace, ["update", &id1, "--priority", "0"], "update_p0");
    let _ = run_obr(&workspace, ["update", &id2, "--priority", "1"], "update_p1");
    let _ = run_obr(&workspace, ["update", &id3, "--priority", "2"], "update_p2");

    let output = run_obr(&workspace, ["ready"], "ready_text");
    assert!(output.status.success(), "ready failed: {}", output.stderr);
    assert_snapshot!("ready_output", normalize_output(&output.stdout));
}

#[test]
fn snapshot_blocked_output() {
    let workspace = init_workspace();

    // Create dependency chain
    let blocker = create_issue(&workspace, "Database schema", "create_blocker");
    let blocked1 = create_issue(&workspace, "User model", "create_blocked1");
    let blocked2 = create_issue(&workspace, "Auth module", "create_blocked2");

    let _ = run_obr(&workspace, ["dep", "add", &blocked1, &blocker], "dep_add1");
    let _ = run_obr(&workspace, ["dep", "add", &blocked2, &blocked1], "dep_add2");

    let output = run_obr(&workspace, ["blocked"], "blocked_text");
    assert!(output.status.success(), "blocked failed: {}", output.stderr);
    assert_snapshot!("blocked_output", normalize_output(&output.stdout));
}

#[test]
fn snapshot_stats_output() {
    let workspace = init_workspace();

    // Create mixed state issues
    let id1 = create_issue(&workspace, "Open issue 1", "create_open1");
    let id2 = create_issue(&workspace, "Open issue 2", "create_open2");
    let id3 = create_issue(&workspace, "Will close", "create_close");

    // Close one issue
    let _ = run_obr(&workspace, ["close", &id3], "close_issue");

    // Add a dependency
    let _ = run_obr(&workspace, ["dep", "add", &id2, &id1], "dep_add_stats");

    let output = run_obr(&workspace, ["stats"], "stats_text");
    assert!(output.status.success(), "stats failed: {}", output.stderr);
    assert_snapshot!("stats_output", normalize_output(&output.stdout));
}

/// A `Cargo.toml` that is deliberately NOT obr's.
///
/// `check_binary_version_mismatch` walks upward from the workspace directory
/// and stops at the first manifest it can parse a `[package].name` out of,
/// flagging only when that name is `obr`. Planting a foreign manifest at the
/// workspace root therefore pins the check to its "no obr Cargo.toml
/// reachable" branch on every host — including rch workers, which put `TMPDIR`
/// inside the synced obr checkout and would otherwise take the in-tree branch.
///
/// This determinism is what lets the golden compare that check's message
/// verbatim instead of masking the whole line. The mask is not a theoretical
/// loss: it is why `no obr Cargo.toml reachable from .beads/` survived four
/// rename passes in shipped plain, rich and `--json` output.
const FOREIGN_CARGO_TOML: &str = "[package]\nname = \"not-obr\"\nversion = \"0.0.0\"\n";

#[test]
fn snapshot_doctor_output() {
    let workspace = ObrWorkspace::new();
    std::fs::write(workspace.root.join("Cargo.toml"), FOREIGN_CARGO_TOML)
        .expect("plant foreign Cargo.toml");
    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success());

    let output = run_obr(&workspace, ["doctor"], "doctor");
    let normalized = normalize_output(&output.stdout);

    // The golden below pins the whole message, but assert the noun explicitly
    // too: a future re-baseline can silently bake a wrong directory name back
    // into the .snap file, and this line cannot be re-baselined.
    assert!(
        normalized.contains(&format!(
            "no obr Cargo.toml reachable from {}/",
            obr::config::WORKSPACE_DIR_NAME
        )),
        "binary_version must name the workspace directory that actually exists:\n{normalized}"
    );

    assert_snapshot!("doctor_output", normalized);
}

/// `binary_version`'s message is not merely decorative: it lands in
/// `checks[].message` of `obr doctor --json`, so a hardcoded workspace noun is
/// a false statement in machine-readable output that agents consume. Assert
/// against `config::WORKSPACE_DIR_NAME` rather than a literal, so renaming the
/// workspace directory again moves the test and the code together.
#[test]
fn doctor_json_binary_version_names_the_real_workspace_dir() {
    let workspace = ObrWorkspace::new();
    std::fs::write(workspace.root.join("Cargo.toml"), FOREIGN_CARGO_TOML)
        .expect("plant foreign Cargo.toml");
    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let output = run_obr(&workspace, ["doctor", "--json"], "doctor_json");
    let report: serde_json::Value =
        serde_json::from_str(&output.stdout).expect("doctor --json emits JSON");

    let message = report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|check| check["name"] == "binary_version")
        .expect("binary_version check present")["message"]
        .as_str()
        .expect("binary_version carries a message")
        .to_string();

    assert!(
        message.ends_with(&format!(
            "no obr Cargo.toml reachable from {}/ — not flagging",
            obr::config::WORKSPACE_DIR_NAME
        )),
        "binary_version message must name the resolved workspace directory, got: {message}"
    );
    // The workspace on disk really is the directory the message names.
    assert!(
        workspace
            .root
            .join(obr::config::WORKSPACE_DIR_NAME)
            .is_dir(),
        "the asserted directory must be the one init created"
    );
}

#[test]
fn snapshot_version_output() {
    let workspace = ObrWorkspace::new();
    let output = run_obr(&workspace, ["version"], "version");
    assert_snapshot!("version_output", normalize_output(&output.stdout));
}

#[test]
fn snapshot_reopen_output() {
    let workspace = init_workspace();
    let id = create_issue(&workspace, "Issue to reopen", "create_for_reopen");

    // Close the issue first
    let close = run_obr(&workspace, ["close", &id], "close_for_reopen");
    assert!(close.status.success(), "close failed: {}", close.stderr);

    // Now reopen it
    let output = run_obr(&workspace, ["reopen", &id], "reopen");
    assert!(output.status.success(), "reopen failed: {}", output.stderr);
    assert_snapshot!("reopen_output", normalize_output(&output.stdout));
}

#[test]
fn snapshot_search_output() {
    let workspace = init_workspace();

    // Create issues with searchable content
    create_issue(&workspace, "Authentication bug in login", "create_search1");
    create_issue(&workspace, "Payment processing feature", "create_search2");
    create_issue(&workspace, "User login flow improvement", "create_search3");

    // Search for "login"
    let output = run_obr(&workspace, ["search", "login"], "search_login");
    assert!(output.status.success(), "search failed: {}", output.stderr);
    assert_snapshot!("search_output", normalize_output(&output.stdout));
}

#[test]
fn snapshot_count_output() {
    let workspace = init_workspace();

    // Create issues with different statuses and types
    let id1 = create_issue(&workspace, "Bug one", "create_count1");
    let id2 = create_issue(&workspace, "Bug two", "create_count2");
    let id3 = create_issue(&workspace, "Feature one", "create_count3");

    // Update types and close one
    let _ = run_obr(
        &workspace,
        ["update", &id1, "--type", "bug"],
        "update_count1",
    );
    let _ = run_obr(
        &workspace,
        ["update", &id2, "--type", "bug"],
        "update_count2",
    );
    let _ = run_obr(
        &workspace,
        ["update", &id3, "--type", "feature"],
        "update_count3",
    );
    let _ = run_obr(&workspace, ["close", &id2], "close_count2");

    let output = run_obr(&workspace, ["count"], "count_text");
    assert!(output.status.success(), "count failed: {}", output.stderr);
    assert_snapshot!("count_output", normalize_output(&output.stdout));
}

#[test]
fn snapshot_label_add_list_output() {
    let workspace = init_workspace();

    // Create an issue and add labels
    let id = create_issue(&workspace, "Issue with labels", "create_label");

    // Add labels
    let add1 = run_obr(&workspace, ["label", "add", &id, "urgent"], "label_add1");
    assert!(add1.status.success(), "label add failed: {}", add1.stderr);

    let add2 = run_obr(&workspace, ["label", "add", &id, "backend"], "label_add2");
    assert!(add2.status.success(), "label add failed: {}", add2.stderr);

    // List labels
    let output = run_obr(&workspace, ["label", "list", &id], "label_list");
    assert!(
        output.status.success(),
        "label list failed: {}",
        output.stderr
    );
    assert_snapshot!("label_list_output", normalize_output(&output.stdout));
}
