//! Round-trip regression over the real 549-issue tracker corpus.
//!
//! `tests/fixtures/corpus/tracker_549.jsonl` is the fork's own issue history
//! (recovered from git history after the in-repo tracker was retired). It is
//! the single most adversarial input the Org layer has faced: descriptions
//! that quote git conflict markers, pasted code under list bullets (the
//! unbounded +2-spaces-per-cycle drift), trailing-whitespace shapes, empty
//! Go-style zero-value optionals, multi-line close reasons, colon-bearing
//! labels, and one tombstone. Every one of those classes produced a real
//! emission bug during the port; this test keeps them fixed.

mod common;

use common::cli::{ObrWorkspace, WORKSPACE_DIR, run_obr};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const CORPUS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/corpus/tracker_549.jsonl"
);
const CORPUS_RECORDS: usize = 549;

/// Point `metadata.json` back at the defaulted surface after a pinned import,
/// removing the pinned artifact so the surface rule (not rule-2b legacy
/// discovery) governs the flush that follows.
fn unpin_to_surface(obr_dir: &Path) {
    let metadata_path = obr_dir.join("metadata.json");
    let text = fs::read_to_string(&metadata_path).expect("read metadata.json");
    let mut value: serde_json::Value = serde_json::from_str(&text).expect("parse metadata.json");
    value["jsonl_export"] = serde_json::Value::String("PLAN.org".to_string());
    fs::write(
        &metadata_path,
        serde_json::to_string_pretty(&value).expect("serialize metadata.json"),
    )
    .expect("write metadata.json");
    let pinned = obr_dir.join("issues.jsonl");
    if pinned.exists() {
        fs::remove_file(&pinned).expect("remove pinned jsonl");
    }
}

#[test]
fn real_corpus_549_org_roundtrip_is_a_fixpoint() {
    let workspace = ObrWorkspace::new();
    let obr_dir = workspace.root.join(WORKSPACE_DIR);

    let init = run_obr(&workspace, ["init", "--prefix", "corpus"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Class A import: pin to JSONL, seed the corpus, import all 549 records.
    common::cli::pin_jsonl(&obr_dir);
    fs::copy(CORPUS, obr_dir.join("issues.jsonl")).expect("seed corpus");
    let import = run_obr(&workspace, ["sync", "--import-only"], "import-corpus");
    assert!(import.status.success(), "import failed: {}", import.stderr);

    // Flip to the default surface and flush the corpus as Org.
    unpin_to_surface(&obr_dir);
    let flush = run_obr(
        &workspace,
        ["sync", "--flush-only", "--force"],
        "flush-gen1",
    );
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let surface = obr::config::computed_surface_path(&workspace.root);
    let gen1 = fs::read_to_string(&surface).expect("read gen1 surface");

    let headings = gen1.lines().filter(|l| l.starts_with("* ")).count();
    assert_eq!(
        headings, CORPUS_RECORDS,
        "every corpus record must become a top-level heading"
    );

    // The sync safety layer's marker scan reads raw lines; a quoted conflict
    // marker that reaches column zero poisons every future import.
    for (idx, line) in gen1.lines().enumerate() {
        assert!(
            !line.starts_with("<<<<<<<")
                && !line.starts_with("=======")
                && !line.starts_with(">>>>>>>"),
            "raw conflict marker at surface line {}: {line}",
            idx + 1
        );
    }

    // Fixpoint: import the surface into a SECOND, empty workspace and flush
    // it there — byte-identical from the FIRST write. This is the assertion
    // that caught the unbounded indentation drift, the empty-property churn,
    // and the trailing-newline wrap/unwrap oscillation.
    //
    // The second workspace is what makes it an assertion at all. Re-importing
    // into the same workspace short-circuits: the gen1 flush stores the
    // surface's content hash, and `sync --import-only` without `--force`
    // skips an import whose source hash equals the stored one, so the Org
    // parser never ran and gen2 was gen1 by construction. A fresh database
    // has no such hash, and gen2 is genuinely parse-then-emit.
    let mirror = ObrWorkspace::new();
    let mirror_dir = mirror.root.join(WORKSPACE_DIR);
    let mirror_init = run_obr(&mirror, ["init", "--prefix", "corpus"], "init-mirror");
    assert!(
        mirror_init.status.success(),
        "mirror init failed: {}",
        mirror_init.stderr
    );
    let mirror_surface = obr::config::computed_surface_path(&mirror.root);
    for seed in [
        mirror_surface.clone(),
        mirror_dir.join("issues.org"),
        mirror_dir.join("issues.jsonl"),
    ] {
        if seed.exists() {
            fs::remove_file(&seed).expect("remove mirror seed");
        }
    }
    fs::copy(&surface, &mirror_surface).expect("copy gen1 surface");

    let mirror_import = run_obr(&mirror, ["sync", "--import-only"], "import-gen1");
    assert!(
        mirror_import.status.success(),
        "mirror import failed: {}",
        mirror_import.stderr
    );
    let mirror_flush = run_obr(&mirror, ["sync", "--flush-only", "--force"], "flush-gen2");
    assert!(
        mirror_flush.status.success(),
        "mirror flush failed: {}",
        mirror_flush.stderr
    );
    let gen2 = fs::read_to_string(&mirror_surface).expect("read gen2 surface");
    assert_eq!(
        gen1, gen2,
        "corpus surface must be a byte-identical fixpoint from the first write"
    );

    // Byte equality proves the file is stable; it does not prove the records
    // survived. Dump both databases and compare them field by field, which
    // also fails loudly if the import above had been a no-op.
    let source_records = dump_records(&workspace, &obr_dir, "dump-source");
    let mirror_records = dump_records(&mirror, &mirror_dir, "dump-mirror");
    assert_eq!(
        source_records.len(),
        CORPUS_RECORDS,
        "the source database must hold the whole corpus"
    );
    assert_eq!(
        source_records.keys().collect::<Vec<_>>(),
        mirror_records.keys().collect::<Vec<_>>(),
        "the Org round trip must preserve the exact id set"
    );

    let mut diffs = Vec::new();
    let mut normalized_count = 0usize;
    let mut truncated_count = 0usize;
    for (id, source) in &source_records {
        let mirror = &mirror_records[id];
        for field in [
            "title",
            "status",
            "priority",
            "issue_type",
            "assignee",
            "labels",
        ] {
            if source.get(field) != mirror.get(field) {
                diffs.push(format!(
                    "{id}.{field}: {:?} != {:?}",
                    source.get(field),
                    mirror.get(field)
                ));
            }
        }
        // Instants are compared under the surface's one documented
        // normalization: an Org timestamp is `[YYYY-MM-DD Ddd HH:MM]`, so the
        // first flush floors every instant to the minute. The mirror must
        // hold exactly that, and must already be a fixpoint of it.
        for field in ["created_at", "updated_at", "closed_at"] {
            let source_time = source
                .get(field)
                .and_then(Value::as_str)
                .map(floor_to_minute);
            let mirror_time = mirror
                .get(field)
                .and_then(Value::as_str)
                .map(floor_to_minute);
            let mirror_raw = mirror.get(field).and_then(Value::as_str);
            if source_time != mirror_time {
                diffs.push(format!(
                    "{id}.{field}: {:?} != {:?}",
                    source.get(field),
                    mirror.get(field)
                ));
            } else if source.get(field).and_then(Value::as_str) != mirror_raw {
                truncated_count += 1;
            }
            if let Some(raw) = mirror_raw
                && floor_to_minute(raw) != raw
            {
                diffs.push(format!(
                    "{id}.{field}: stored form is not minute-precision: {raw:?}"
                ));
            }
        }
        // Free text is compared under exactly one documented normalization —
        // the surface cannot carry edge blank lines — and the mirror side
        // must already be a fixpoint of it. Anything the Org layer loses
        // beyond that class shows up here.
        for field in [
            "description",
            "design",
            "acceptance_criteria",
            "notes",
            "close_reason",
        ] {
            let source_text = source.get(field).and_then(Value::as_str);
            let mirror_text = mirror.get(field).and_then(Value::as_str);
            if source_text.map(canonical_body) != mirror_text.map(canonical_body) {
                diffs.push(format!("{id}.{field}: {source_text:?} != {mirror_text:?}"));
            } else if source_text != mirror_text {
                normalized_count += 1;
            }
            if let Some(text) = mirror_text
                && canonical_body(text) != text
            {
                diffs.push(format!(
                    "{id}.{field}: stored form is not canonical: {text:?}"
                ));
            }
        }
        let deps = |record: &Value| {
            record
                .get("dependencies")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        };
        if deps(source) != deps(mirror) {
            diffs.push(format!(
                "{id}.dependencies: {} != {}",
                deps(source),
                deps(mirror)
            ));
        }
    }
    assert!(
        diffs.is_empty(),
        "field-level drift across the corpus round trip ({} findings over {CORPUS_RECORDS} \
         records):\n{}",
        diffs.len(),
        diffs
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    // The corpus does exercise the normalization (22 descriptions end in a
    // newline); if it ever stopped, this test would be asserting nothing
    // about that class and the guard belongs somewhere else.
    assert!(
        normalized_count > 0,
        "the corpus no longer exercises the one-time text normalization"
    );
    // Likewise for the timestamp truncation: the corpus is full of
    // sub-minute instants, and if it ever stopped being, this test would be
    // asserting nothing about the precision the surface actually carries.
    assert!(
        truncated_count > 0,
        "the corpus no longer exercises the one-time timestamp truncation"
    );
}

/// The Org surface's storage form for an instant, stated independently of the
/// implementation it checks (`org_bridge::format_org_timestamp`): an Org
/// timestamp is `[YYYY-MM-DD Ddd HH:MM]`, so seconds and sub-seconds are
/// dropped on the first write and minute precision becomes the data model.
///
/// The rendering is local, but every zone in the IANA database has a
/// whole-minute UTC offset in the modern era, so flooring the UTC instant to
/// the minute is the same operation as flooring the local reading.
fn floor_to_minute(rfc3339: &str) -> String {
    use chrono::Timelike;
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .unwrap_or_else(|e| panic!("record timestamp {rfc3339:?} is not RFC3339: {e}"))
        .with_timezone(&chrono::Utc)
        .with_second(0)
        .expect("second 0 is always valid")
        .with_nanosecond(0)
        .expect("nanosecond 0 is always valid")
        // Match how the JSONL export renders a `DateTime<Utc>`, so the
        // comparison stays string-to-string with the exported values.
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The Org surface's storage form for free text, stated independently of the
/// implementation it checks (`org_bridge::canonical_body_text`): CRLF folds
/// to LF, trailing newlines and leading blank lines are dropped, because a
/// blank line is what delimits a body on the surface.
fn canonical_body(text: &str) -> String {
    let lf = text.replace("\r\n", "\n");
    let body = lf.trim_end_matches('\n');
    let lines: Vec<&str> = body.split('\n').collect();
    let first = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    lines[first..].join("\n")
}

/// Dump a workspace's whole database as JSONL, keyed by issue id.
///
/// Pinning the workspace to JSONL and flushing is the only export that
/// carries every field of every record — `list` projects, and `show` per id
/// would be 549 processes. Destructive to the workspace's surface
/// configuration, so callers must already have read anything they need.
fn dump_records(workspace: &ObrWorkspace, obr_dir: &Path, label: &str) -> BTreeMap<String, Value> {
    common::cli::pin_jsonl(obr_dir);
    let flush = run_obr(workspace, ["sync", "--flush-only", "--force"], label);
    assert!(flush.status.success(), "{label} failed: {}", flush.stderr);
    let text = fs::read_to_string(obr_dir.join("issues.jsonl")).expect("read jsonl dump");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: Value = serde_json::from_str(line).expect("parse jsonl record");
            let id = value["id"].as_str().expect("record id").to_string();
            (id, value)
        })
        .collect()
}
