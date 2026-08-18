//! The Org round-trip gate: the Org surface is a lossless representation of
//! the database. Anything obr can store must survive a flush and come back
//! identical, with exactly one documented normalization — the seven instant
//! fields floor to the minute, the precision an Org timestamp can carry.
//!
//! init → create issues carrying labels, dependencies, comments and non-ASCII
//! text → flush → the surface parses as well-formed Org → re-import into a
//! fresh DB → field-level equality against the originals. A field that stops
//! round-tripping is a regression, not a known loss to be widened for.
//!
//! Two adjacent guarantees ride along, because they are the ways this same
//! invariant rots without any single field going missing: flushing from
//! unchanged state must be deterministic, and `--no-db` must not manufacture
//! phantom changes in blocks it did not touch.

mod common;

use common::cli::{ObrWorkspace, export_path, run_obr};
use serde_json::Value;
use std::fs;

fn parse_created_id(stdout: &str) -> String {
    // Rich/plain create output contains "Created issue: <id>" or the id as
    // the first token of a line like "<id> [P1] ..."; the robust route is
    // the first token that looks like "<prefix>-<suffix>".
    stdout
        .split_whitespace()
        .find(|token| {
            let t = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
            t.contains('-') && !t.starts_with('-') && !t.ends_with('-')
        })
        .map(|token| {
            token
                .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                .to_string()
        })
        .unwrap_or_default()
}

/// The `Issue` fields the Org surface stores as a timestamp property.
const ORG_INSTANT_FIELDS: &[&str] = &[
    "created_at",
    "updated_at",
    "closed_at",
    "due_at",
    "defer_until",
    "deleted_at",
    "compacted_at",
];

/// Floor an RFC3339 instant to the minute — the precision an Org timestamp
/// can carry, stated here independently of the emitter that produces it.
fn floor_to_minute(rfc3339: &str) -> String {
    use chrono::Timelike;
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .unwrap_or_else(|e| panic!("timestamp {rfc3339:?} is not RFC3339: {e}"))
        .with_timezone(&chrono::Utc)
        .with_second(0)
        .expect("second 0 is always valid")
        .with_nanosecond(0)
        .expect("nanosecond 0 is always valid")
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A field value under the surface's one instant normalization; everything
/// that is not one of the seven timestamps is returned untouched.
fn normalize_instant(key: &str, value: Option<&Value>) -> Option<Value> {
    let value = value?;
    if !ORG_INSTANT_FIELDS.contains(&key) {
        return Some(value.clone());
    }
    Some(value.as_str().map_or_else(
        || value.clone(),
        |text| Value::String(floor_to_minute(text)),
    ))
}

fn extract_json_payload(stdout: &str) -> String {
    let start = stdout.find(['[', '{']).unwrap_or(0);
    stdout[start..].to_string()
}

fn show_issue_json(workspace: &ObrWorkspace, id: &str, label: &str) -> Value {
    let show = run_obr(workspace, ["show", id, "--json"], label);
    assert!(show.status.success(), "show {id} failed: {}", show.stderr);
    let payload = extract_json_payload(&show.stdout);
    let mut values: Vec<Value> = serde_json::from_str(&payload).expect("show json");
    assert_eq!(values.len(), 1, "show must return exactly one issue");
    values.remove(0)
}

/// The full Org round-trip gate: field-level equality after re-import into
/// a fresh database.
#[test]
#[allow(clippy::too_many_lines)]
fn org_roundtrip_field_level_equality_into_fresh_db() {
    let source = ObrWorkspace::new();
    let init = run_obr(&source, ["init", "--prefix", "rt"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Unicode title + tag-safe and colon-bearing labels.
    let create = run_obr(
        &source,
        [
            "create",
            "Unicode ★ Ünïcode ❤ test",
            "--type",
            "task",
            "--priority",
            "1",
        ],
        "create_unicode",
    );
    assert!(create.status.success(), "create failed: {}", create.stderr);
    let id_a = parse_created_id(&create.stdout);
    assert!(!id_a.is_empty(), "missing created id: {}", create.stdout);

    let create_b = run_obr(
        &source,
        ["create", "Second issue", "--type", "bug", "--priority", "3"],
        "create_second",
    );
    assert!(
        create_b.status.success(),
        "create b failed: {}",
        create_b.stderr
    );
    let id_b = parse_created_id(&create_b.stdout);

    for label in ["alpha", "provides:parser"] {
        let add = run_obr(&source, ["label", "add", &id_a, label], "label_add");
        assert!(add.status.success(), "label add failed: {}", add.stderr);
    }
    let dep = run_obr(&source, ["dep", "add", &id_b, &id_a], "dep_add");
    assert!(dep.status.success(), "dep add failed: {}", dep.stderr);
    let comment = run_obr(
        &source,
        [
            "comments",
            "add",
            &id_a,
            "A comment with *markup* and\na second line",
        ],
        "comment_add",
    );
    assert!(
        comment.status.success(),
        "comment failed: {}",
        comment.stderr
    );
    let desc = run_obr(
        &source,
        [
            "update",
            &id_a,
            "--description",
            "Body text.\n\n- item one\n- item two\n\n#+begin_src rust\nfn main() {}\n#+end_src",
        ],
        "update_desc",
    );
    assert!(desc.status.success(), "update failed: {}", desc.stderr);

    let flush = run_obr(&source, ["sync", "--flush-only"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    // The export artifact is the tracked surface and is well-formed.
    let org_path = export_path(&source);
    let org_text = fs::read_to_string(&org_path).expect("read the surface");
    assert!(
        org_text.starts_with("#+TITLE:"),
        "missing header: {org_text}"
    );
    assert!(
        org_text.contains(&format!(":ID:       {id_a}")),
        "missing {id_a}"
    );
    assert!(
        org_text.contains(":LABELS:   [\"alpha\",\"provides:parser\"]"),
        "authoritative labels missing: {org_text}"
    );

    // Flush determinism: a second flush is byte-identical.
    let first_bytes = fs::read(&org_path).expect("read bytes");
    let flush2 = run_obr(&source, ["sync", "--flush-only"], "flush2");
    assert!(flush2.status.success(), "flush2 failed: {}", flush2.stderr);
    assert_eq!(
        fs::read(&org_path).expect("re-read bytes"),
        first_bytes,
        "second flush must be byte-identical"
    );

    // Re-import into a fresh database (init-then-copy: the supported flow —
    // fresh clones without any DB fail closed upstream for JSONL too).
    let dest = ObrWorkspace::new();
    let init2 = run_obr(&dest, ["init", "--prefix", "rt"], "init_dest");
    assert!(init2.status.success(), "dest init failed: {}", init2.stderr);
    fs::copy(&org_path, export_path(&dest)).expect("copy the surface");
    let import = run_obr(&dest, ["sync", "--import-only", "--force"], "import_dest");
    assert!(import.status.success(), "import failed: {}", import.stderr);

    // Field-level equality across every synced field.
    let src_issue = show_issue_json(&source, &id_a, "show_src");
    let dst_issue = show_issue_json(&dest, &id_a, "show_dst");
    let mut diffs = Vec::new();
    let keys: std::collections::BTreeSet<&String> = src_issue
        .as_object()
        .unwrap()
        .keys()
        .chain(dst_issue.as_object().unwrap().keys())
        .collect();
    for key in keys {
        // The one field class the surface cannot carry verbatim: an Org
        // timestamp is `[YYYY-MM-DD Ddd HH:MM]`, so the first flush floors
        // every instant to the minute and the re-imported copy holds that.
        // Everything else must be equal byte for byte.
        let src = normalize_instant(key, src_issue.get(key.as_str()));
        let dst = normalize_instant(key, dst_issue.get(key.as_str()));
        if src != dst {
            diffs.push(format!(
                "{key}: {:?} != {:?}",
                src_issue.get(key.as_str()),
                dst_issue.get(key.as_str())
            ));
        }
        if ORG_INSTANT_FIELDS.contains(&key.as_str())
            && let Some(raw) = dst_issue.get(key.as_str()).and_then(Value::as_str)
        {
            assert_eq!(
                floor_to_minute(raw),
                raw,
                "{key} came back from the surface with sub-minute precision"
            );
        }
    }
    assert!(
        diffs.is_empty(),
        "field-level differences after round-trip:\n{}",
        diffs.join("\n")
    );
}

/// Regression: `--no-db show` previously died on Org workspaces because the
/// exact-scan fast path parsed the export as JSONL lines; Org paths must fall
/// through to the format-aware materialized loader.
#[test]
fn no_db_show_reads_an_org_workspace() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init", "--prefix", "ns"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);
    let create = run_obr(
        &workspace,
        ["create", "Shown issue", "--type", "task"],
        "create",
    );
    assert!(create.status.success(), "create failed: {}", create.stderr);
    let id = parse_created_id(&create.stdout);
    let flush = run_obr(&workspace, ["sync", "--flush-only"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let issue = {
        let show = run_obr(&workspace, ["--no-db", "show", &id, "--json"], "no_db_show");
        assert!(
            show.status.success(),
            "--no-db show failed: {}",
            show.stderr
        );
        let payload = extract_json_payload(&show.stdout);
        let mut values: Vec<Value> = serde_json::from_str(&payload).expect("show json");
        values.remove(0)
    };
    assert_eq!(issue["title"], "Shown issue");
}

/// The direct regression test for the fork-era phantom-change failure mode:
/// a `--no-db` mutation must rewrite the export
/// deterministically, changing only the affected issue's block, and an
/// untouched issue's block must survive byte-identically.
#[test]
fn no_db_create_changes_only_one_block() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init", "--prefix", "nd"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let create = run_obr(
        &workspace,
        ["create", "Stable issue", "--type", "task"],
        "create_stable",
    );
    assert!(create.status.success(), "create failed: {}", create.stderr);
    let stable_id = parse_created_id(&create.stdout);
    let flush = run_obr(&workspace, ["sync", "--flush-only"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let org_path = export_path(&workspace);
    let before = fs::read_to_string(&org_path).expect("read before");
    let stable_block_before: Vec<&str> = before
        .lines()
        .skip_while(|l| !l.contains(&format!(":ID:       {stable_id}")))
        .take_while(|l| !l.starts_with("* ") || l.contains("Stable issue"))
        .collect();
    assert!(!stable_block_before.is_empty(), "stable block missing");

    // Mutate via --no-db: seeds the in-memory DB from the tracked surface,
    // creates a new issue, writes the file back.
    let no_db = run_obr(
        &workspace,
        ["--no-db", "create", "No-db issue", "--type", "task"],
        "create_no_db",
    );
    assert!(
        no_db.status.success(),
        "--no-db create failed: {}",
        no_db.stderr
    );

    let after = fs::read_to_string(&org_path).expect("read after");
    assert!(after.contains("No-db issue"), "new issue missing");
    let stable_block_after: Vec<&str> = after
        .lines()
        .skip_while(|l| !l.contains(&format!(":ID:       {stable_id}")))
        .take_while(|l| !l.starts_with("* ") || l.contains("Stable issue"))
        .collect();
    assert_eq!(
        stable_block_before, stable_block_after,
        "an untouched issue's block must survive a --no-db mutation byte-identically"
    );
}

/// Import a surface file into a brand-new workspace and flush it there — the
/// fresh-clone flow (`obr init && obr sync --import-only --rebuild`), and the
/// only way to observe what the parser actually reconstructs.
fn reflush_in_fresh_workspace(surface: &std::path::Path, prefix: &str) -> String {
    let dest = ObrWorkspace::new();
    let init = run_obr(&dest, ["init", "--prefix", prefix], "init_fresh");
    assert!(init.status.success(), "fresh init failed: {}", init.stderr);
    let dest_surface = export_path(&dest);
    if dest_surface.exists() {
        fs::remove_file(&dest_surface).expect("remove seeded surface");
    }
    fs::copy(surface, &dest_surface).expect("copy surface");
    let import = run_obr(&dest, ["sync", "--import-only"], "import_fresh");
    assert!(import.status.success(), "import failed: {}", import.stderr);
    let flush = run_obr(&dest, ["sync", "--flush-only", "--force"], "flush_fresh");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);
    fs::read_to_string(&dest_surface).expect("read regenerated surface")
}

/// A `PLAN.org` written before the Org timestamps existed — `:CREATED_AT:`
/// and friends carrying RFC3339 — must import through the real binary without
/// losing an instant, and come back out in the current form. This is the
/// whole compatibility promise, asserted end to end rather than at the
/// parser.
#[test]
fn a_legacy_rfc3339_surface_imports_and_reexports_as_org_timestamps() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init", "--prefix", "legacy"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let surface = export_path(&workspace);
    fs::write(
        &surface,
        "#+TITLE: Obr Issues\n\
         #+SEQ_TODO: TODO DOING DRAFT WAIT DEFER NOTE | DONE CANCELED\n\
         #+ISSUE_PREFIX: legacy\n\n\
         * DONE [#B] Old surface\n\
         :PROPERTIES:\n\
         :ID:       legacy-1\n\
         :ISSUE_TYPE: task\n\
         :CREATED_AT: 2026-01-02T03:04:00+00:00\n\
         :UPDATED_AT: 2026-01-02T03:05:00+00:00\n\
         :CLOSED_AT: 2026-01-02T03:06:00+00:00\n\
         :DUE_AT: 2026-01-02T03:07:00+00:00\n\
         :DEFER_UNTIL: 2026-01-02T03:08:00+00:00\n\
         :END:\n\n",
    )
    .expect("seed a legacy surface");

    let import = run_obr(&workspace, ["sync", "--import-only", "--force"], "import");
    assert!(import.status.success(), "import failed: {}", import.stderr);

    // Nothing was lost on the way in.
    let issue = show_issue_json(&workspace, "legacy-1", "show");
    for (field, expected) in [
        ("created_at", "2026-01-02T03:04:00Z"),
        ("updated_at", "2026-01-02T03:05:00Z"),
        ("closed_at", "2026-01-02T03:06:00Z"),
        ("due_at", "2026-01-02T03:07:00Z"),
        ("defer_until", "2026-01-02T03:08:00Z"),
    ] {
        let got = issue.get(field).and_then(Value::as_str);
        assert_eq!(
            got.map(floor_to_minute).as_deref(),
            Some(expected),
            "{field} was lost importing a legacy surface: {issue:?}"
        );
    }

    // And it comes back out in the current form only.
    let flush = run_obr(&workspace, ["sync", "--flush-only", "--force"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);
    let gen1 = fs::read_to_string(&surface).expect("read gen1");
    assert!(
        !gen1.contains("_AT:"),
        "a legacy spelling survived:\n{gen1}"
    );
    assert!(!gen1.contains(":DEFER_UNTIL:"), "got:\n{gen1}");
    for key in [
        ":CREATED:",
        ":MODIFIED:",
        ":FINISHED:",
        ":DUE:",
        ":DEFERRED:",
    ] {
        assert!(gen1.contains(key), "{key} missing from:\n{gen1}");
    }
    // The rewrite is itself the fixpoint: no second-flush churn.
    let gen2 = reflush_in_fresh_workspace(&surface, "legacy");
    assert_eq!(
        gen1, gen2,
        "the rewritten surface must be a fixpoint from the first write"
    );
}

/// One unreleased build spelled `closed_at` as `:CLOSED:` — an Org timestamp
/// under a key `org-special-properties` shadows, which is why the property is
/// now `:FINISHED:`. Scratch workspaces written by that build exist, so the
/// close instant must survive the import and come back out under the current
/// key. Asserted through the real binary, not the parser.
#[test]
fn the_retired_closed_property_imports_and_reexports_as_finished() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init", "--prefix", "retired"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let surface = export_path(&workspace);
    fs::write(
        &surface,
        "#+TITLE: Obr Issues\n\
         #+SEQ_TODO: TODO DOING DRAFT WAIT DEFER NOTE | DONE CANCELED\n\
         #+ISSUE_PREFIX: retired\n\n\
         * DONE [#B] Closed under the retired key\n\
         :PROPERTIES:\n\
         :ID:       retired-1\n\
         :ISSUE_TYPE: task\n\
         :CREATED:  [2026-01-02 Fri 03:04]\n\
         :MODIFIED: [2026-01-02 Fri 03:05]\n\
         :CLOSED:   [2026-01-02 Fri 03:06]\n\
         :END:\n\n",
    )
    .expect("seed a surface carrying the retired spelling");

    let import = run_obr(&workspace, ["sync", "--import-only", "--force"], "import");
    assert!(import.status.success(), "import failed: {}", import.stderr);

    // The close instant survived. It is a LOCAL reading, so compare against
    // the other two properties from the same file rather than a fixed UTC
    // string: they are one and two minutes apart by construction.
    let issue = show_issue_json(&workspace, "retired-1", "show");
    let stamp = |field: &str| {
        let text = issue
            .get(field)
            .and_then(Value::as_str)
            .map(floor_to_minute)
            .unwrap_or_else(|| panic!("{field} missing from {issue:?}"));
        chrono::DateTime::parse_from_rfc3339(&text).expect("floor_to_minute emits RFC3339")
    };
    assert_eq!(
        stamp("closed_at") - stamp("created_at"),
        chrono::TimeDelta::minutes(2),
        "the retired :CLOSED: instant was lost or shifted: {issue:?}"
    );

    // And it comes back out under the current key only.
    let flush = run_obr(&workspace, ["sync", "--flush-only", "--force"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);
    let gen1 = fs::read_to_string(&surface).expect("read gen1");
    assert!(
        gen1.contains(":FINISHED: [2026-01-02 Fri 03:06]"),
        "the close instant did not re-export as :FINISHED::\n{gen1}"
    );
    assert!(
        !gen1.contains(":CLOSED:") && !gen1.contains(":CLOSED_AT:"),
        "a retired spelling survived:\n{gen1}"
    );

    let gen2 = reflush_in_fresh_workspace(&surface, "retired");
    assert_eq!(
        gen1, gen2,
        "the rewritten surface must be a fixpoint from the first write"
    );
}

/// A deleted issue's tombstone must survive the fresh-clone round trip
/// byte-identically. The delete leaves `closed_at` NULL — `deleted_at` is the
/// tombstone's timestamp, and the schema CHECK exempts tombstones — but the
/// import used to back-fill `closed_at` for any terminal status, so the
/// rebuilt workspace grew a `:FINISHED:` line the original flush never
/// wrote.
#[test]
fn deleted_issue_roundtrips_without_gaining_closed_at() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init", "--prefix", "tomb"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let create = run_obr(
        &workspace,
        ["create", "Doomed issue", "--type", "task"],
        "create",
    );
    assert!(create.status.success(), "create failed: {}", create.stderr);
    let id = parse_created_id(&create.stdout);

    let delete = run_obr(
        &workspace,
        ["delete", &id, "--reason", "not needed", "--force"],
        "delete",
    );
    assert!(delete.status.success(), "delete failed: {}", delete.stderr);

    let flush = run_obr(&workspace, ["sync", "--flush-only", "--force"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);
    let surface = export_path(&workspace);
    let gen1 = fs::read_to_string(&surface).expect("read gen1");
    assert!(gen1.contains("* CANCELED"), "tombstone missing: {gen1}");
    assert!(
        !gen1.contains(":FINISHED:") && !gen1.contains(":CLOSED:") && !gen1.contains(":CLOSED_AT:"),
        "a delete must not set closed_at: {gen1}"
    );

    let gen2 = reflush_in_fresh_workspace(&surface, "tomb");
    assert_eq!(
        gen1, gen2,
        "a tombstone must round-trip through a fresh workspace byte-identically"
    );
}

/// `PLAN.org` is the file users are told to edit, so a section obr cannot
/// store has to be announced — the next flush rewrites the file from the
/// database and the text is gone. The warning goes to stderr and leaves
/// stdout machine-parseable under `--json`.
#[test]
fn unrecognized_org_section_warns_without_breaking_json_stdout() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init", "--prefix", "sect"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);
    let create = run_obr(
        &workspace,
        ["create", "Hand-edited issue", "--type", "task"],
        "create",
    );
    assert!(create.status.success(), "create failed: {}", create.stderr);
    let id = parse_created_id(&create.stdout);
    let flush = run_obr(&workspace, ["sync", "--flush-only", "--force"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let surface = export_path(&workspace);
    let text = fs::read_to_string(&surface).expect("read surface");
    fs::write(
        &surface,
        format!("{text}** Scratch notes\nkeep this somehow\n\n"),
    )
    .expect("hand-edit surface");

    let import = run_obr(
        &workspace,
        ["--json", "sync", "--import-only", "--force"],
        "import_edited",
    );
    assert!(import.status.success(), "import failed: {}", import.stderr);
    assert!(
        import.stderr.contains("UNRECOGNIZED_ORG_SECTION"),
        "warning code missing from stderr: {}",
        import.stderr
    );
    assert!(
        import.stderr.contains(&id) && import.stderr.contains("Scratch notes"),
        "warning must name the issue and the section: {}",
        import.stderr
    );
    let warning = import
        .stderr
        .lines()
        .find(|line| line.contains("UNRECOGNIZED_ORG_SECTION"))
        .expect("warning line");
    let payload: Value = serde_json::from_str(warning.trim()).expect("warning is a JSON envelope");
    assert_eq!(payload["warning"]["code"], "UNRECOGNIZED_ORG_SECTION");

    // stdout stays parseable: the warning never leaves stderr.
    let list = run_obr(&workspace, ["list", "--json"], "list_json");
    assert!(list.status.success(), "list failed: {}", list.stderr);
    let _: Value =
        serde_json::from_str(&extract_json_payload(&list.stdout)).expect("stdout stays JSON");
}

/// A hand-edit that breaks a reserved section's JSON block must fail the
/// import loudly. Importing it as empty and flushing deleted every stored
/// dependency, silently, on the file the product tells users to edit.
#[test]
fn hand_edited_dependencies_section_without_json_is_refused() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init", "--prefix", "deps"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);
    let a = parse_created_id(
        &run_obr(
            &workspace,
            ["create", "Blocked work", "--type", "task"],
            "a",
        )
        .stdout,
    );
    let b = parse_created_id(
        &run_obr(
            &workspace,
            ["create", "Blocking work", "--type", "task"],
            "b",
        )
        .stdout,
    );
    let dep = run_obr(&workspace, ["dep", "add", &a, &b], "dep_add");
    assert!(dep.status.success(), "dep add failed: {}", dep.stderr);
    let flush = run_obr(&workspace, ["sync", "--flush-only", "--force"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let surface = export_path(&workspace);
    let text = fs::read_to_string(&surface).expect("read surface");
    assert!(text.contains("** Dependencies"), "no deps section: {text}");
    let broken = text.replace("#+begin_src json", "#+begin_src JSON");
    fs::write(&surface, &broken).expect("hand-edit surface");

    let import = run_obr(
        &workspace,
        ["sync", "--import-only", "--force"],
        "import_broken",
    );
    assert!(
        !import.status.success(),
        "a broken reserved section must fail the import: {}",
        import.stdout
    );
    assert!(
        import.stderr.contains("Dependencies") && import.stderr.contains(&a),
        "error must name the section and the issue: {}",
        import.stderr
    );

    // The refusal did not touch the database: restore the block the way a
    // user would and the edge is still there. (Every command that reads the
    // surface fails while the file is broken, which is the point.)
    fs::write(&surface, &text).expect("restore surface");
    let show = show_issue_json(&workspace, &a, "show_after_refusal");
    assert_eq!(
        show["dependencies"].as_array().map_or(0, Vec::len),
        1,
        "the refused import must leave the dependency intact: {show}"
    );
}

/// A hand edit made in Emacs survives, even when nothing bumps `:MODIFIED:`
/// and the edit touches only fields the content hash cannot see.
///
/// The Org surface renders `:MODIFIED:` at minute precision, so a record read
/// back from the file is strictly older than the sub-minute instant the
/// database holds for the same write, and last-write-wins answered "existing
/// is newer" on every hand edit — skipping it, then reverting the file on the
/// next flush. Timestamps are now compared at the surface's own precision, and
/// the resulting tie is decided by `sync_equals`, which covers labels,
/// dependencies and comments as well as the scalar fields.
///
/// Labels are the case that matters here: they are absent from `content_hash`,
/// so editing a headline's tags — the everyday Emacs edit — is exactly what a
/// content-hash-only tie-break misses.
#[test]
fn a_hand_edit_with_no_modified_bump_survives_import_and_reflush() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init", "--prefix", "hand"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let created = run_obr(
        &workspace,
        ["create", "Edit me in Emacs", "-t", "task", "-l", "alpha"],
        "create",
    );
    assert!(
        created.status.success(),
        "create failed: {}",
        created.stderr
    );
    let flush = run_obr(&workspace, ["sync", "--flush-only"], "flush");
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let surface = export_path(&workspace);
    let before = fs::read_to_string(&surface).expect("read surface");
    let modified_before = before
        .lines()
        .find(|line| line.starts_with(":MODIFIED:"))
        .expect("surface carries :MODIFIED:")
        .to_string();

    // Two edits a person would make by hand, neither touching :MODIFIED:.
    // The title is covered by content_hash; the tags are not.
    let edited = before
        .replace("Edit me in Emacs", "Edited in Emacs")
        .replace(":alpha:", ":alpha:urgent:")
        .replace(r#"["alpha"]"#, r#"["alpha","urgent"]"#);
    assert_ne!(before, edited, "the edit must actually change the file");
    fs::write(&surface, &edited).expect("write the hand edit");

    let import = run_obr(&workspace, ["sync", "--import-only"], "import");
    assert!(import.status.success(), "import failed: {}", import.stderr);

    let after = fs::read_to_string(&surface).expect("read surface");
    assert!(
        after.contains("Edited in Emacs"),
        "the title edit was reverted:\n{after}"
    );
    assert!(
        after.contains(":alpha:urgent:"),
        "the tag edit was reverted -- labels are invisible to content_hash:\n{after}"
    );
    assert!(
        after.contains(&modified_before),
        ":MODIFIED: must not be fabricated by the import:\n{after}"
    );

    // And the surface is still a fixpoint: re-importing changes nothing.
    let reimport = run_obr(&workspace, ["sync", "--import-only"], "reimport");
    assert!(
        reimport.status.success(),
        "reimport failed: {}",
        reimport.stderr
    );
    assert_eq!(
        fs::read_to_string(&surface).expect("read surface"),
        after,
        "a second import of an unchanged surface must not rewrite it"
    );
}
