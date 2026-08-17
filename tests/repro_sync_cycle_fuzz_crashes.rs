//! Regression tests for sync_cycle fuzz crash artifacts.
//!
//! Each test feeds a minimized crash input through the same sync infrastructure
//! exercised by the fuzz harness. The goal is to verify these inputs no longer
//! cause panics after the c0f7749 hardening commit, and to prevent regressions.

mod common;

use obr::model::{Issue, IssueType, Priority, Status};
use obr::storage::SqliteStorage;
use obr::sync::{
    ExportConfig, ImportConfig, OrphanMode, compute_jsonl_hash, compute_staleness,
    ensure_no_conflict_markers, export_to_jsonl, import_from_jsonl, preflight_import,
};
use std::fs;
use tempfile::TempDir;

fn setup_workspace() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let temp = TempDir::new().expect("temp dir");
    let obr_dir = temp.path().join(".obr");
    fs::create_dir_all(&obr_dir).expect("create .obr");
    let jsonl_path = obr_dir.join("issues.jsonl");
    (temp, obr_dir, jsonl_path)
}

fn seed_storage(storage: &mut SqliteStorage) {
    let issue = Issue {
        id: "bd-sync-0".to_string(),
        title: "Seed issue".to_string(),
        status: Status::Open,
        priority: Priority(2),
        issue_type: IssueType::Task,
        created_by: Some("test".to_string()),
        ..Issue::default()
    };
    storage.create_issue(&issue, "test").unwrap();
}

fn default_export_config(obr_dir: &std::path::Path) -> ExportConfig {
    ExportConfig {
        force: true,
        is_default_path: true,
        obr_dir: Some(obr_dir.to_path_buf()),
        show_progress: false,
        ..ExportConfig::default()
    }
}

fn default_import_config(obr_dir: &std::path::Path) -> ImportConfig {
    ImportConfig {
        skip_prefix_validation: true,
        clear_duplicate_external_refs: true,
        obr_dir: Some(obr_dir.to_path_buf()),
        show_progress: false,
        ..ImportConfig::default()
    }
}

/// crash-1ea9: Git merge conflict markers in JSONL.
/// The sync infrastructure must detect conflict markers and reject the file
/// rather than panicking.
#[test]
fn repro_sync_cycle_crash_conflict_markers() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).unwrap();
    seed_storage(&mut storage);

    let conflict_content = b"<<<<<<< HEAD\n{\"id\":\"bd-conflict\",\"title\":\"left\"}\n=======\n{\"id\":\"bd-conflict\",\"title\":\"right\"}\n>>>>>>> branch\n";
    fs::write(&jsonl_path, conflict_content).unwrap();

    let conflict_result = ensure_no_conflict_markers(&jsonl_path);
    assert!(
        conflict_result.is_err(),
        "conflict markers should be rejected"
    );

    let import_result = import_from_jsonl(
        &mut storage,
        &jsonl_path,
        &default_import_config(&obr_dir),
        Some("bd"),
    );
    match import_result {
        Ok(_) => panic!("import should reject conflict-marked JSONL"),
        Err(err) => assert!(
            !err.to_string().trim().is_empty(),
            "error message should not be empty"
        ),
    }
}

/// crash-51295: Git conflict markers with embedded content strings.
/// Variant of the conflict marker test with longer content inside the markers.
#[test]
fn repro_sync_cycle_crash_conflict_markers_variant() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).unwrap();
    seed_storage(&mut storage);

    let conflict_content = b"<<<<<<< HEAD\n{\"id\":\"bd-conflict\",\"title\":\"leftnl-export-import-reop\"}\n=======\n{\"id\":\"bd-conflict\",\"title\":\"right\"}\n>>>>>>> branch\n";
    fs::write(&jsonl_path, conflict_content).unwrap();

    let conflict_result = ensure_no_conflict_markers(&jsonl_path);
    assert!(
        conflict_result.is_err(),
        "conflict markers should be rejected"
    );
}

/// crash-02bf: Fuzz input that triggers specific export/import/upsert paths.
/// This 48-byte input is decoded by the ByteCursor and drives the sync cycle
/// through force-export, force-upsert, and orphan-mode-variant paths.
#[test]
fn repro_sync_cycle_crash_export_upsert_orphan() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).unwrap();
    seed_storage(&mut storage);

    export_to_jsonl(&storage, &jsonl_path, &default_export_config(&obr_dir)).unwrap();

    let jsonl_content = fs::read_to_string(&jsonl_path).unwrap();
    let modified = format!(
        "{jsonl_content}\n{{\"id\":\"bd-orphan-test\",\"title\":\"orphan issue\",\"status\":\"open\",\"priority\":2,\"issue_type\":\"task\"}}\n"
    );
    fs::write(&jsonl_path, modified).unwrap();

    let config_strict = ImportConfig {
        orphan_mode: OrphanMode::Strict,
        ..default_import_config(&obr_dir)
    };
    let _ = import_from_jsonl(&mut storage, &jsonl_path, &config_strict, Some("bd"));

    let config_allow = ImportConfig {
        orphan_mode: OrphanMode::Allow,
        ..default_import_config(&obr_dir)
    };
    let _ = import_from_jsonl(&mut storage, &jsonl_path, &config_allow, Some("bd"));

    let config_skip = ImportConfig {
        orphan_mode: OrphanMode::Skip,
        ..default_import_config(&obr_dir)
    };
    let _ = import_from_jsonl(&mut storage, &jsonl_path, &config_skip, Some("bd"));

    let config_resurrect = ImportConfig {
        orphan_mode: OrphanMode::Resurrect,
        ..default_import_config(&obr_dir)
    };
    let _ = import_from_jsonl(&mut storage, &jsonl_path, &config_resurrect, Some("bd"));
}

/// crash-8533: Binary/null bytes in the byte stream combined with force-export
/// and upsert flags.
#[test]
fn repro_sync_cycle_crash_binary_garbage_jsonl() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).unwrap();
    seed_storage(&mut storage);

    export_to_jsonl(&storage, &jsonl_path, &default_export_config(&obr_dir)).unwrap();

    let mut content = fs::read(&jsonl_path).unwrap();
    content.extend_from_slice(b"\n\x00\x06garbage-line-with-nulls\n");
    fs::write(&jsonl_path, &content).unwrap();

    let import_result = import_from_jsonl(
        &mut storage,
        &jsonl_path,
        &default_import_config(&obr_dir),
        Some("bd"),
    );
    match import_result {
        Ok(_) => {}
        Err(err) => assert!(
            !err.to_string().trim().is_empty(),
            "error message should not be empty"
        ),
    }
}

/// crash-f397: Highly mangled binary data mixed with partial string fragments.
#[test]
fn repro_sync_cycle_crash_mangled_binary() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).unwrap();
    seed_storage(&mut storage);

    export_to_jsonl(&storage, &jsonl_path, &default_export_config(&obr_dir)).unwrap();

    let mangled: Vec<u8> = vec![
        0xbb, 0x61, 0x55, 0x04, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x55, 0x64, 0x74, 0x40, 0x72,
        0x69, 0x49, 0x4d, 0x4d, 0x45, 0x44, 0xf3, 0xf3, 0xf3, 0xf3, 0xf3, 0xf3, 0xf3,
    ];
    fs::write(&jsonl_path, &mangled).unwrap();

    let import_result = import_from_jsonl(
        &mut storage,
        &jsonl_path,
        &default_import_config(&obr_dir),
        Some("bd"),
    );
    match import_result {
        Ok(_) => {}
        Err(err) => assert!(
            !err.to_string().trim().is_empty(),
            "error message should not be empty"
        ),
    }

    let staleness = compute_staleness(&storage, &jsonl_path);
    match staleness {
        Ok(_) => {}
        Err(err) => assert!(!err.to_string().trim().is_empty()),
    }
}

/// slow-unit: Input that triggers a slow path in the sync cycle.
/// Verify it completes within a reasonable timeout.
#[test]
fn repro_sync_cycle_slow_unit_force_upsert_orphan() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).unwrap();
    seed_storage(&mut storage);

    export_to_jsonl(&storage, &jsonl_path, &default_export_config(&obr_dir)).unwrap();

    let config_force = ImportConfig {
        force_upsert: true,
        orphan_mode: OrphanMode::Allow,
        ..default_import_config(&obr_dir)
    };

    let start = std::time::Instant::now();
    let _ = import_from_jsonl(&mut storage, &jsonl_path, &config_force, Some("bd"));
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 10,
        "force-upsert import took too long: {elapsed:?}"
    );
}

/// Verify CRLF line endings in JSONL don't cause panics.
#[test]
fn repro_sync_cycle_crlf_jsonl() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).unwrap();
    seed_storage(&mut storage);

    export_to_jsonl(&storage, &jsonl_path, &default_export_config(&obr_dir)).unwrap();

    let content = fs::read_to_string(&jsonl_path).unwrap();
    let crlf_content = content.replace('\n', "\r\n");
    fs::write(&jsonl_path, crlf_content).unwrap();

    let import_result = import_from_jsonl(
        &mut storage,
        &jsonl_path,
        &default_import_config(&obr_dir),
        Some("bd"),
    );
    match import_result {
        Ok(_) => {}
        Err(err) => assert!(!err.to_string().trim().is_empty()),
    }
}

/// Verify duplicate JSONL lines are handled gracefully.
#[test]
fn repro_sync_cycle_duplicate_lines() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).unwrap();
    seed_storage(&mut storage);

    export_to_jsonl(&storage, &jsonl_path, &default_export_config(&obr_dir)).unwrap();

    let content = fs::read_to_string(&jsonl_path).unwrap();
    if let Some(first_line) = content.lines().find(|l| !l.trim().is_empty()) {
        let duplicated = format!("{content}\n{first_line}\n");
        fs::write(&jsonl_path, duplicated).unwrap();
    }

    let _ = import_from_jsonl(
        &mut storage,
        &jsonl_path,
        &default_import_config(&obr_dir),
        Some("bd"),
    );
}

/// Verify empty JSONL file doesn't crash.
#[test]
fn repro_sync_cycle_empty_jsonl() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).unwrap();
    seed_storage(&mut storage);

    fs::write(&jsonl_path, "").unwrap();

    let import_result = import_from_jsonl(
        &mut storage,
        &jsonl_path,
        &default_import_config(&obr_dir),
        Some("bd"),
    );
    match import_result {
        Ok(_) => {}
        Err(err) => assert!(!err.to_string().trim().is_empty()),
    }
}

/// Verify corrupt SQLite DB doesn't cause panics in sync paths.
#[test]
fn repro_sync_cycle_corrupt_db() {
    let (_temp, obr_dir, _jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");

    fs::write(&db_path, b"not a sqlite database\n").unwrap();

    let storage_result = SqliteStorage::open(&db_path);
    match storage_result {
        Ok(_) => {
            panic!("corrupt DB should fail to open or at least be detected during operations");
        }
        Err(err) => assert!(
            !err.to_string().trim().is_empty(),
            "corrupt DB error should have a message"
        ),
    }
}

/// Verify stale WAL/SHM sidecar files don't cause panics.
#[test]
fn repro_sync_cycle_stale_wal_shm() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();
    let db_path = obr_dir.join("obr.db");

    fs::write(obr_dir.join("obr.db-wal"), b"garbage wal data").unwrap();
    fs::write(obr_dir.join("obr.db-shm"), b"garbage shm data").unwrap();

    let storage_result = SqliteStorage::open(&db_path);
    match storage_result {
        Ok(mut storage) => {
            seed_storage(&mut storage);
            let export_result =
                export_to_jsonl(&storage, &jsonl_path, &default_export_config(&obr_dir));
            match export_result {
                Ok(_) => {}
                Err(err) => assert!(!err.to_string().trim().is_empty()),
            }
        }
        Err(err) => assert!(!err.to_string().trim().is_empty()),
    }
}

/// Verify preflight_import handles malformed JSONL gracefully.
#[test]
fn repro_sync_cycle_preflight_malformed() {
    let (_temp, obr_dir, jsonl_path) = setup_workspace();

    fs::write(&jsonl_path, b"{bad json\n{also bad\n").unwrap();

    let preflight_result =
        preflight_import(&jsonl_path, &default_import_config(&obr_dir), Some("bd"));
    match preflight_result {
        Ok(_) => {}
        Err(err) => assert!(!err.to_string().trim().is_empty()),
    }
}

/// Verify compute_jsonl_hash handles empty and malformed files.
#[test]
fn repro_sync_cycle_hash_edge_cases() {
    let (_temp, _obr_dir, jsonl_path) = setup_workspace();

    fs::write(&jsonl_path, "").unwrap();
    let hash1 = compute_jsonl_hash(&jsonl_path);
    assert!(hash1.is_ok(), "empty file should hash successfully");

    fs::write(&jsonl_path, "not json at all\n").unwrap();
    let hash2 = compute_jsonl_hash(&jsonl_path);
    assert!(hash2.is_ok(), "non-json file should still hash the bytes");
}
