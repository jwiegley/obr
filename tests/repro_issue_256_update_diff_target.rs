//! Regression test for issue #256: `obr update` stdout prints an unrelated
//! bead's fields as the diff.
//!
//! Symptom (quoted from the reporter):
//! ```
//! $ obr update <target-id> --priority 1
//! Updated <target-id>: <UNRELATED BEAD'S TITLE>
//!   status: open → closed
//!   priority: P1 → P2
//!   type: bug → task
//! ```
//!
//! The target bead's on-disk state is unchanged, and `obr show <target-id>`
//! returns the correct values; only the "Updated …" diff block printed to
//! stdout references a different bead's title + fields.
//!
//! The root cause is that the post-write display path used a second
//! `get_issue(id)` read to render the diff.  A rare fsqlite read-path
//! inconsistency (prepared-statement / pager cache edge case) can make that
//! second read return data that belongs to a different row while the write
//! itself is correct.
//!
//! The fix is to stop trusting a second read for rendering: the diff is now
//! synthesized from the validated pre-mutation `issue_before` snapshot
//! (whose `id` equality is guarded by `get_issue_from_conn`'s post-condition
//! check) and the exact `IssueUpdate` the user asked for.  As a defensive
//! consequence: (a) the header "Updated <id>: <title>" always references
//! the target bead, and (b) no diff line can appear for a field the user
//! did not explicitly request to change.

mod common;

use common::cli::{ObrWorkspace, parse_created_id, run_obr};

/// Minimal end-to-end guarantee: when we update the **target** bead, the
/// "Updated <id>: <title>" header must name the target bead's title, never
/// an unrelated bead's title.  This is the primary regression the reporter
/// observed in issue #256.
#[test]
fn br_update_prints_target_beads_title_not_unrelated_bead_title() {
    let _log = common::test_log("repro_issue_256_header_title");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Create a "noise" bead first — a closed P2 task, exactly matching the
    // shape of the unrelated bead whose fields were leaking into the diff
    // in the bug report.  Closing immediately ensures it is terminal state.
    let noise_create = run_obr(
        &workspace,
        [
            "create",
            "UNRELATED NOISE BEAD TITLE",
            "--type",
            "task",
            "--priority",
            "2",
        ],
        "create_noise",
    );
    assert!(
        noise_create.status.success(),
        "noise create failed: {}",
        noise_create.stderr
    );
    let noise_id = parse_created_id(&noise_create.stdout);
    let noise_close = run_obr(&workspace, ["close", &noise_id], "close_noise");
    assert!(
        noise_close.status.success(),
        "noise close failed: {}",
        noise_close.stderr
    );

    // Create the target bead: open / P1 / bug, exactly matching the
    // reporter's scenario.
    let target_title = "TARGET BEAD TITLE";
    let target_create = run_obr(
        &workspace,
        ["create", target_title, "--type", "bug", "--priority", "1"],
        "create_target",
    );
    assert!(
        target_create.status.success(),
        "target create failed: {}",
        target_create.stderr
    );
    let target_id = parse_created_id(&target_create.stdout);

    // Run the reporter's exact command: idempotent `--priority 1` on an
    // already-P1 bead.
    let update = run_obr(
        &workspace,
        ["update", &target_id, "--priority", "1"],
        "update_priority_noop",
    );
    assert!(update.status.success(), "update failed: {}", update.stderr);

    // Assertion 1: the "Updated" header must reference the target bead's
    // title, not the noise bead's title.  This directly catches the
    // "unrelated bead's title appears in Updated line" regression.
    assert!(
        update.stdout.contains(target_title),
        "update stdout must reference target bead title {target_title:?}; got: {:?}",
        update.stdout,
    );
    assert!(
        !update.stdout.contains("UNRELATED NOISE BEAD TITLE"),
        "update stdout must NOT reference the unrelated noise bead's title; got: {:?}",
        update.stdout,
    );

    // Assertion 2: because `--priority 1` is a no-op on a P1 bead, the diff
    // block must not contain any status / type / priority transition lines.
    // These are the exact ghost-field lines the reporter saw leaking in.
    for ghost in [
        "status: open → closed",
        "priority: P1 → P2",
        "type: bug → task",
    ] {
        assert!(
            !update.stdout.contains(ghost),
            "update stdout must not contain ghost diff line {ghost:?} for a no-op \
             --priority 1 on an already-P1 bead; got: {:?}",
            update.stdout,
        );
    }

    // Assertion 3: the underlying data on disk must remain target's
    // original values (the reporter confirmed this; we re-assert as a
    // positive invariant).
    let show = run_obr(&workspace, ["show", &target_id, "--json"], "show_after");
    assert!(show.status.success(), "show failed: {}", show.stderr);
    let payload = common::cli::extract_json_payload(&show.stdout);
    let show_json: Vec<serde_json::Value> = serde_json::from_str(&payload).expect("show json");
    assert_eq!(show_json[0]["id"], target_id);
    assert_eq!(show_json[0]["title"], target_title);
    assert_eq!(show_json[0]["status"], "open");
    assert_eq!(show_json[0]["priority"], 1);
    assert_eq!(show_json[0]["issue_type"], "bug");
}

/// A real (non-noop) update must produce a correctly-attributed diff block:
/// the header title matches the target bead, and the printed before/after
/// values match the user's requested change — not some other bead's fields.
#[test]
fn obr_update_prints_correct_diff_for_real_field_change() {
    let _log = common::test_log("repro_issue_256_real_change");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Same noise shape as above.
    let noise_create = run_obr(
        &workspace,
        [
            "create",
            "ANOTHER UNRELATED NOISE BEAD",
            "--type",
            "task",
            "--priority",
            "2",
        ],
        "create_noise",
    );
    let noise_id = parse_created_id(&noise_create.stdout);
    let _ = run_obr(&workspace, ["close", &noise_id], "close_noise");

    let target_title = "REAL CHANGE TARGET";
    let target_create = run_obr(
        &workspace,
        ["create", target_title, "--type", "bug", "--priority", "1"],
        "create_target",
    );
    let target_id = parse_created_id(&target_create.stdout);

    // Request a genuine priority change (P1 -> P3).
    let update = run_obr(
        &workspace,
        ["update", &target_id, "--priority", "3"],
        "update_real_change",
    );
    assert!(update.status.success(), "update failed: {}", update.stderr);

    // Header references the target bead.
    assert!(
        update.stdout.contains(target_title),
        "update stdout must reference target title {target_title:?}; got: {:?}",
        update.stdout,
    );

    // Diff contains exactly the requested transition.
    assert!(
        update.stdout.contains("priority: P1 → P3"),
        "update stdout must print the requested priority transition; got: {:?}",
        update.stdout,
    );

    // Diff does not include unrequested ghost fields.
    assert!(
        !update.stdout.contains("status: open → closed"),
        "update stdout must not include a ghost status transition; got: {:?}",
        update.stdout,
    );
    assert!(
        !update.stdout.contains("type: bug → task"),
        "update stdout must not include a ghost type transition; got: {:?}",
        update.stdout,
    );
}
