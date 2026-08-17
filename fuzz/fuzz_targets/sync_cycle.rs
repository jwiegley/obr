#![no_main]

mod common;

use obr::model::{DependencyType, Issue, Priority, Status};
use obr::storage::SqliteStorage;
use obr::sync::{
    ExportConfig, ImportConfig, OrphanMode, compute_jsonl_hash, compute_staleness,
    ensure_no_conflict_markers, export_to_jsonl, import_from_jsonl, preflight_import,
};
use obr::validation::IssueValidator;
use chrono::Utc;
use common::{BuiltInIssueCursorExt, ByteCursor};
use libfuzzer_sys::fuzz_target;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::{Builder, TempDir};

const MAX_INPUT_BYTES: usize = 16 * 1024;
const MAX_FIELD_BYTES: usize = 96;
const MAX_MUTATION_BYTES: usize = 512;
const ACTOR: &str = "sync-cycle-fuzz";

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }

    let result = run_sync_cycle_case(data);
    assert!(
        result.is_ok(),
        "sync cycle fuzz invariant failed: {result:?}"
    );
});

fn run_sync_cycle_case(data: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut cursor = ByteCursor::new(data);
    let workspace = FuzzWorkspace::new(&mut cursor)?;
    let db_path = workspace.obr_dir.join("obr.db");
    let jsonl_path = choose_jsonl_path(&workspace.obr_dir, &mut cursor);
    if let Some(parent) = jsonl_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let wrote_db_sidecars = write_sidecar_files(&workspace.obr_dir, &mut cursor)?;
    let corrupt_db_family = wrote_db_sidecars
        || maybe_write_corrupt_db_family(&workspace.obr_dir, &db_path, &mut cursor)?;

    {
        let mut storage = match SqliteStorage::open(&db_path) {
            Ok(storage) => storage,
            Err(err) if corrupt_db_family => {
                assert_nonempty_error(err)?;
                workspace.assert_outside_untouched()?;
                return Ok(());
            }
            Err(err) => return Err(err.into()),
        };

        if let Err(err) = seed_storage(&mut storage, &mut cursor) {
            if corrupt_db_family {
                assert_nonempty_error(err.to_string())?;
                workspace.assert_outside_untouched()?;
                return Ok(());
            }
            return Err(err);
        }
        if assert_storage_invariants_or_expected(&storage, corrupt_db_family, &workspace)? {
            return Ok(());
        }

        exercise_rejected_paths(&storage, &workspace)?;

        let export_config = export_config(&workspace.obr_dir, &jsonl_path, &mut cursor);
        match export_to_jsonl(&storage, &jsonl_path, &export_config) {
            Ok(_) => {
                assert_jsonl_parseable(&jsonl_path)?;
                maybe_save_base_snapshot(&workspace.obr_dir, &jsonl_path, &mut cursor)?;
            }
            Err(err) => assert_nonempty_error(err)?,
        }

        mutate_jsonl(&jsonl_path, &mut cursor)?;

        if jsonl_path.exists() {
            allow_error(ensure_no_conflict_markers(&jsonl_path))?;
            allow_error(preflight_import(
                &jsonl_path,
                &import_config(&workspace.obr_dir, &mut cursor),
                Some("bd"),
            ))?;
            allow_error(compute_staleness(&storage, &jsonl_path))?;
            allow_error(compute_jsonl_hash(&jsonl_path))?;

            let import_result = import_from_jsonl(
                &mut storage,
                &jsonl_path,
                &import_config(&workspace.obr_dir, &mut cursor),
                Some("bd"),
            );
            match import_result {
                Ok(_) => {
                    if assert_storage_invariants_or_expected(
                        &storage,
                        corrupt_db_family,
                        &workspace,
                    )? {
                        return Ok(());
                    }
                    assert_import_validation(&storage)?;
                }
                Err(err) => {
                    assert_nonempty_error(err)?;
                    if assert_storage_invariants_or_expected(
                        &storage,
                        corrupt_db_family,
                        &workspace,
                    )? {
                        return Ok(());
                    }
                }
            }
        }

        let forced_export = export_to_jsonl(
            &storage,
            &jsonl_path,
            &ExportConfig {
                force: true,
                is_default_path: true,
                obr_dir: Some(workspace.obr_dir.clone()),
                show_progress: false,
                ..ExportConfig::default()
            },
        );
        match forced_export {
            Ok(_) => {
                assert_jsonl_parseable(&jsonl_path)?;
                allow_error(compute_staleness(&storage, &jsonl_path))?;
            }
            Err(err) => assert_nonempty_error(err)?,
        }

        if assert_storage_invariants_or_expected(&storage, corrupt_db_family, &workspace)? {
            return Ok(());
        }
    }

    let reopened = match SqliteStorage::open(&db_path) {
        Ok(storage) => storage,
        Err(err) if corrupt_db_family => {
            assert_nonempty_error(err)?;
            workspace.assert_outside_untouched()?;
            return Ok(());
        }
        Err(err) => return Err(err.into()),
    };
    if assert_storage_invariants_or_expected(&reopened, corrupt_db_family, &workspace)? {
        return Ok(());
    }

    if jsonl_path.exists() && cursor.next_bool() {
        let mut rebuilt = SqliteStorage::open(&workspace.obr_dir.join("rebuild-cycle.db"))?;
        match import_from_jsonl(
            &mut rebuilt,
            &jsonl_path,
            &import_config(&workspace.obr_dir, &mut cursor),
            Some("bd"),
        ) {
            Ok(_) => {
                if assert_storage_invariants_or_expected(&rebuilt, corrupt_db_family, &workspace)? {
                    return Ok(());
                }
                assert_import_validation(&rebuilt)?;
            }
            Err(err) => assert_nonempty_error(err)?,
        }
    }

    workspace.assert_outside_untouched()?;
    Ok(())
}

struct FuzzWorkspace {
    _temp: TempDir,
    obr_dir: PathBuf,
    outside_dir: PathBuf,
    outside_sentinel: PathBuf,
    outside_contents: String,
}

impl FuzzWorkspace {
    fn new(cursor: &mut ByteCursor<'_>) -> Result<Self, Box<dyn Error>> {
        let temp = Builder::new().prefix("br-sync-cycle-fuzz").tempdir()?;
        let workspace_dir = temp.path().join("workspace");
        fs::create_dir(&workspace_dir)?;

        let outside_dir = temp.path().join("outside");
        fs::create_dir(&outside_dir)?;
        let outside_sentinel = outside_dir.join("sentinel.txt");
        let outside_contents = "outside sentinel\n".to_string();
        fs::write(&outside_sentinel, &outside_contents)?;

        let obr_dir = workspace_dir.join(".obr");
        create_obr_dir(temp.path(), &obr_dir, cursor.next_bool())?;

        Ok(Self {
            _temp: temp,
            obr_dir,
            outside_dir,
            outside_sentinel,
            outside_contents,
        })
    }

    fn assert_outside_untouched(&self) -> Result<(), Box<dyn Error>> {
        let actual = fs::read_to_string(&self.outside_sentinel)?;
        if actual != self.outside_contents {
            return Err("outside sentinel was modified".into());
        }

        for entry in fs::read_dir(&self.outside_dir)? {
            let path = entry?.path();
            if path != self.outside_sentinel {
                return Err(
                    format!("unexpected write outside obr dir: {}", path.display()).into(),
                );
            }
        }

        Ok(())
    }
}

#[cfg(unix)]
fn create_obr_dir(
    temp_root: &Path,
    obr_dir: &Path,
    symlinked: bool,
) -> Result<(), Box<dyn Error>> {
    if symlinked {
        let target = temp_root.join("linked-beads");
        fs::create_dir(&target)?;
        std::os::unix::fs::symlink(&target, obr_dir)?;
    } else {
        fs::create_dir(obr_dir)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_obr_dir(
    _temp_root: &Path,
    obr_dir: &Path,
    _symlinked: bool,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir(obr_dir)?;
    Ok(())
}

fn choose_jsonl_path(obr_dir: &Path, cursor: &mut ByteCursor<'_>) -> PathBuf {
    match cursor.next_byte() % 4 {
        0 => obr_dir.join("issues.jsonl"),
        1 => obr_dir.join("custom-sync.jsonl"),
        2 => obr_dir.join("nested").join("issues.jsonl"),
        _ => obr_dir.join("nested").join("deeper").join("issues.jsonl"),
    }
}

fn write_sidecar_files(
    obr_dir: &Path,
    cursor: &mut ByteCursor<'_>,
) -> Result<bool, Box<dyn Error>> {
    let sidecars = [
        "beads.db-wal",
        "beads.db-shm",
        "beads.base.jsonl",
        "beads.left.jsonl",
        "beads.right.jsonl",
    ];
    let mut wrote_db_sidecars = false;

    for sidecar in sidecars {
        if cursor.next_bool() {
            fs::write(obr_dir.join(sidecar), cursor.bytes(96))?;
            if matches!(sidecar, "beads.db-wal" | "beads.db-shm") {
                wrote_db_sidecars = true;
            }
        }
    }

    if cursor.next_bool() {
        let recovery_dir = obr_dir.join(".br_recovery");
        fs::create_dir_all(&recovery_dir)?;
        fs::write(recovery_dir.join("candidate.jsonl"), cursor.bytes(96))?;
    }

    Ok(wrote_db_sidecars)
}

fn maybe_write_corrupt_db_family(
    obr_dir: &Path,
    db_path: &Path,
    cursor: &mut ByteCursor<'_>,
) -> Result<bool, Box<dyn Error>> {
    match cursor.next_byte() % 6 {
        0 => Ok(false),
        1 => {
            fs::write(db_path, b"not a sqlite database\n")?;
            Ok(true)
        }
        2 => {
            fs::write(db_path, cursor.bytes(192))?;
            Ok(true)
        }
        3 => {
            fs::write(db_path, b"SQLite format 3\0truncated")?;
            fs::write(obr_dir.join("beads.db-wal"), cursor.bytes(128))?;
            Ok(true)
        }
        4 => {
            fs::write(db_path, [])?;
            fs::write(obr_dir.join("beads.db-shm"), cursor.bytes(128))?;
            Ok(true)
        }
        _ => {
            fs::write(db_path, b"SQLite format 3\0")?;
            fs::write(obr_dir.join("beads.db-wal"), cursor.bytes(256))?;
            fs::write(obr_dir.join("beads.db-shm"), cursor.bytes(256))?;
            Ok(true)
        }
    }
}

fn seed_storage(
    storage: &mut SqliteStorage,
    cursor: &mut ByteCursor<'_>,
) -> Result<(), Box<dyn Error>> {
    let count = 1 + cursor.usize(5);
    let mut ids: Vec<String> = Vec::with_capacity(count);
    let mut relation_ids: Vec<String> = Vec::with_capacity(count);

    for index in 0..count {
        let id = format!("bd-sync-{index}");
        let issue = issue_from_cursor(id.clone(), cursor, index);
        let can_relate = !matches!(issue.status, Status::Tombstone);
        storage.create_issue(&issue, ACTOR)?;

        if can_relate && cursor.next_bool() {
            storage.add_label(&id, &format!("label-{}", cursor.next_byte() % 8), ACTOR)?;
        }
        if can_relate && !relation_ids.is_empty() && cursor.next_bool() {
            let parent = relation_ids[cursor.usize(relation_ids.len())].as_str();
            storage.add_dependency(&id, parent, DependencyType::Blocks.as_str(), ACTOR)?;
        }
        if can_relate && cursor.next_bool() {
            let comment = non_empty(cursor.text(MAX_FIELD_BYTES), "sync fuzz comment");
            storage.add_comment(&id, ACTOR, &comment)?;
        }

        ids.push(id);
        if can_relate {
            relation_ids.push(format!("bd-sync-{index}"));
        }
    }

    Ok(())
}

fn issue_from_cursor(id: String, cursor: &mut ByteCursor<'_>, index: usize) -> Issue {
    let status = cursor.status();
    let closed_at = if matches!(status, Status::Closed) {
        Some(Utc::now())
    } else {
        None
    };

    Issue {
        id,
        title: non_empty(
            cursor.text(MAX_FIELD_BYTES),
            &format!("sync fuzz issue {index}"),
        ),
        description: cursor.optional_text(MAX_FIELD_BYTES),
        design: cursor.optional_text(MAX_FIELD_BYTES),
        acceptance_criteria: cursor.optional_text(MAX_FIELD_BYTES),
        notes: cursor.optional_text(MAX_FIELD_BYTES),
        status,
        priority: Priority(i32::from(cursor.next_byte() % 5)),
        issue_type: cursor.issue_type(),
        assignee: cursor.optional_text(32),
        owner: cursor.optional_text(32),
        created_by: Some(ACTOR.to_string()),
        external_ref: if cursor.next_bool() {
            Some(format!("fuzz-ext-{index}-{}", cursor.next_byte()))
        } else {
            None
        },
        source_system: if cursor.next_bool() {
            Some("fuzz-sync-cycle".to_string())
        } else {
            None
        },
        pinned: cursor.next_bool(),
        is_template: cursor.next_bool(),
        closed_at,
        ..Issue::default()
    }
}

fn export_config(obr_dir: &Path, jsonl_path: &Path, cursor: &mut ByteCursor<'_>) -> ExportConfig {
    ExportConfig {
        force: cursor.next_bool(),
        is_default_path: jsonl_path.file_name().and_then(|name| name.to_str())
            == Some("issues.jsonl"),
        obr_dir: Some(obr_dir.to_path_buf()),
        allow_external_jsonl: false,
        show_progress: false,
        ..ExportConfig::default()
    }
}

fn import_config(obr_dir: &Path, cursor: &mut ByteCursor<'_>) -> ImportConfig {
    ImportConfig {
        skip_prefix_validation: cursor.next_bool(),
        rename_on_import: false,
        clear_duplicate_external_refs: true,
        orphan_mode: match cursor.next_byte() % 4 {
            0 => OrphanMode::Strict,
            1 => OrphanMode::Allow,
            2 => OrphanMode::Skip,
            _ => OrphanMode::Resurrect,
        },
        force_upsert: cursor.next_bool(),
        obr_dir: Some(obr_dir.to_path_buf()),
        allow_external_jsonl: false,
        show_progress: false,
    }
}

fn mutate_jsonl(path: &Path, cursor: &mut ByteCursor<'_>) -> Result<(), Box<dyn Error>> {
    if !path.exists() {
        return Ok(());
    }

    match cursor.next_byte() % 8 {
        0 => {}
        1 => {
            let mut content = fs::read_to_string(path).unwrap_or_default();
            content.push_str("\n\n  \n");
            fs::write(path, content)?;
        }
        2 => {
            let mut content = fs::read(path)?;
            content.extend_from_slice(b"\n");
            content.extend_from_slice(&cursor.bytes(MAX_MUTATION_BYTES));
            content.extend_from_slice(b"\n");
            fs::write(path, content)?;
        }
        3 => {
            fs::write(
                path,
                b"<<<<<<< HEAD\n{\"id\":\"bd-conflict\",\"title\":\"left\"}\n=======\n{\"id\":\"bd-conflict\",\"title\":\"right\"}\n>>>>>>> branch\n",
            )?;
        }
        4 => rewrite_generated_jsonl(path, cursor)?,
        5 => append_duplicate_line(path)?,
        6 => {
            let content = fs::read_to_string(path).unwrap_or_default();
            fs::write(path, content.replace('\n', "\r\n"))?;
        }
        _ => fs::write(path, [])?,
    }

    Ok(())
}

fn rewrite_generated_jsonl(path: &Path, cursor: &mut ByteCursor<'_>) -> Result<(), Box<dyn Error>> {
    let count = 1 + cursor.usize(4);
    let mut lines = Vec::with_capacity(count);
    for index in 0..count {
        let issue = issue_from_cursor(format!("bd-json-{index}"), cursor, index);
        lines.push(serde_json::to_string(&issue)?);
    }
    fs::write(path, format!("{}\n", lines.join("\n")))?;
    Ok(())
}

fn append_duplicate_line(path: &Path) -> Result<(), Box<dyn Error>> {
    let content = fs::read_to_string(path).unwrap_or_default();
    let Some(first_line) = content.lines().find(|line| !line.trim().is_empty()) else {
        return Ok(());
    };
    fs::write(path, format!("{content}\n{first_line}\n"))?;
    Ok(())
}

fn maybe_save_base_snapshot(
    obr_dir: &Path,
    jsonl_path: &Path,
    cursor: &mut ByteCursor<'_>,
) -> Result<(), Box<dyn Error>> {
    if cursor.next_bool() {
        let content = fs::read(jsonl_path)?;
        fs::write(obr_dir.join("beads.base.jsonl"), content)?;
    }
    Ok(())
}

fn exercise_rejected_paths(
    storage: &SqliteStorage,
    workspace: &FuzzWorkspace,
) -> Result<(), Box<dyn Error>> {
    let rejected_git_path = workspace.obr_dir.join(".git").join("issues.jsonl");
    let config = ExportConfig {
        force: true,
        obr_dir: Some(workspace.obr_dir.clone()),
        show_progress: false,
        ..ExportConfig::default()
    };

    match export_to_jsonl(storage, &rejected_git_path, &config) {
        Ok(_) => {
            return Err(format!(
                "export unexpectedly accepted git path: {}",
                rejected_git_path.display()
            )
            .into());
        }
        Err(err) => assert_nonempty_error(err)?,
    }
    if rejected_git_path.exists() {
        return Err(format!(
            "rejected git path was created: {}",
            rejected_git_path.display()
        )
        .into());
    }

    let external_path = workspace.outside_dir.join("direct-external.jsonl");
    match export_to_jsonl(storage, &external_path, &config) {
        Ok(_) => {
            return Err(format!(
                "export unexpectedly accepted external path: {}",
                external_path.display()
            )
            .into());
        }
        Err(err) => assert_nonempty_error(err)?,
    }
    if external_path.exists() {
        return Err(format!(
            "rejected external path was created: {}",
            external_path.display()
        )
        .into());
    }

    exercise_symlink_escape_path(storage, workspace, &config)
}

#[cfg(unix)]
fn exercise_symlink_escape_path(
    storage: &SqliteStorage,
    workspace: &FuzzWorkspace,
    config: &ExportConfig,
) -> Result<(), Box<dyn Error>> {
    let outside_target = workspace.outside_dir.join("escape.jsonl");
    let link_path = workspace.obr_dir.join("escape.jsonl");
    if !link_path.exists() {
        std::os::unix::fs::symlink(&outside_target, &link_path)?;
    }

    match export_to_jsonl(storage, &link_path, config) {
        Ok(_) => {
            if outside_target.exists() {
                return Err("export through symlink wrote outside sentinel directory".into());
            }
        }
        Err(err) => assert_nonempty_error(err)?,
    }

    Ok(())
}

#[cfg(not(unix))]
fn exercise_symlink_escape_path(
    _storage: &SqliteStorage,
    _workspace: &FuzzWorkspace,
    _config: &ExportConfig,
) -> Result<(), Box<dyn Error>> {
    Ok(())
}

fn assert_jsonl_parseable(path: &Path) -> Result<usize, Box<dyn Error>> {
    let content = fs::read_to_string(path)?;
    let mut count = 0usize;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let _: Issue = serde_json::from_str(trimmed)?;
        count += 1;
    }
    Ok(count)
}

fn assert_storage_invariants(storage: &SqliteStorage) -> Result<(), Box<dyn Error>> {
    let ids: HashSet<String> = storage.get_all_ids()?.into_iter().collect();

    for issue in storage.get_all_issues_for_export()? {
        if !ids.contains(&issue.id) {
            return Err(format!("export returned issue absent from id index: {}", issue.id).into());
        }
    }

    for (issue_id, labels) in storage.get_all_labels()? {
        if !ids.contains(&issue_id) {
            return Err(format!("label row references missing issue: {issue_id}").into());
        }
        let mut unique_labels = HashSet::new();
        for label in labels {
            if !unique_labels.insert(label.clone()) {
                return Err(format!("duplicate label for {issue_id}: {label}").into());
            }
        }
    }

    for (issue_id, comments) in storage.get_all_comments()? {
        if !ids.contains(&issue_id) {
            return Err(format!("comment row references missing issue: {issue_id}").into());
        }
        for comment in comments {
            if comment.issue_id != issue_id {
                return Err(format!(
                    "comment map key {issue_id} disagrees with comment issue_id {}",
                    comment.issue_id
                )
                .into());
            }
        }
    }

    for (issue_id, dependencies) in storage.get_all_dependency_records()? {
        if !ids.contains(&issue_id) {
            return Err(
                format!("dependency row references missing source issue: {issue_id}").into(),
            );
        }
        let mut unique_edges = HashSet::new();
        for dependency in dependencies {
            if dependency.issue_id != issue_id {
                return Err(format!(
                    "dependency map key {issue_id} disagrees with dependency issue_id {}",
                    dependency.issue_id
                )
                .into());
            }
            if !dependency.depends_on_id.starts_with("external:")
                && !ids.contains(&dependency.depends_on_id)
            {
                return Err(format!(
                    "dependency from {issue_id} references missing target {}",
                    dependency.depends_on_id
                )
                .into());
            }
            let edge = (dependency.issue_id, dependency.depends_on_id);
            if !unique_edges.insert(edge.clone()) {
                return Err(format!("duplicate dependency edge: {edge:?}").into());
            }
        }
    }

    Ok(())
}

fn assert_storage_invariants_or_expected(
    storage: &SqliteStorage,
    expected_storage_error: bool,
    workspace: &FuzzWorkspace,
) -> Result<bool, Box<dyn Error>> {
    match assert_storage_invariants(storage) {
        Ok(()) => Ok(false),
        Err(err) if expected_storage_error => {
            assert_nonempty_error(err.to_string())?;
            workspace.assert_outside_untouched()?;
            Ok(true)
        }
        Err(err) => Err(err),
    }
}

fn allow_error<T, E>(result: Result<T, E>) -> Result<Option<T>, Box<dyn Error>>
where
    E: Display,
{
    match result {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            assert_nonempty_error(err)?;
            Ok(None)
        }
    }
}

fn assert_nonempty_error<E>(err: E) -> Result<(), Box<dyn Error>>
where
    E: Display,
{
    if err.to_string().trim().is_empty() {
        return Err("operation returned an empty error message".into());
    }
    Ok(())
}

fn assert_import_validation(storage: &SqliteStorage) -> Result<(), Box<dyn Error>> {
    for issue in storage.get_all_issues_for_export()? {
        if let Err(errors) = IssueValidator::validate(&issue) {
            let import_violations: Vec<_> = errors
                .into_iter()
                .filter(|e| {
                    matches!(
                        e.field.as_str(),
                        "title" | "description" | "estimated_minutes"
                    )
                })
                .collect();
            if !import_violations.is_empty() {
                return Err(format!(
                    "imported issue {} has validation failures: {import_violations:?}",
                    issue.id
                )
                .into());
            }
        }
    }
    Ok(())
}

fn non_empty(value: String, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}
