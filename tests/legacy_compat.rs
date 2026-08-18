//! Legacy-name compatibility coverage for the `beads`/`bd`/`br` → `obr`
//! rename.
//!
//! Every test here pins one row of the rename map — the row IDs (`a1`…`a17`,
//! `e*`, `m1`) live on in the test names — and asserts the same three things:
//! the pre-rename name still works, it produces exactly one deprecation
//! warning on stderr, and the *current* name is what gets written.
//!
//! Deprecation warnings are once-per-process, so every assertion about warning
//! counts runs the binary in a fresh subprocess with a scrubbed environment.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

/// A project root with a fully controlled environment.
///
/// `HOME` points inside the sandbox so the user-config chain is hermetic, and
/// every `BD_`/`BEADS_`/`BR_`/`OBR_` variable inherited from the developer's
/// shell is removed before each run.
struct Sandbox {
    _temp: TempDir,
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let temp = TempDir::new().expect("tempdir");
        // macOS hands out /var/folders/... which is a symlink to /private/var;
        // resolve it now so path comparisons against the binary's output hold.
        let root = dunce::canonicalize(temp.path()).expect("canonicalize tempdir");
        Self { _temp: temp, root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Run `obr` in the sandbox with no legacy or current env vars set.
    fn run(&self, args: &[&str]) -> Run {
        self.run_with_env(&[], args)
    }

    fn run_with_env(&self, env: &[(&str, &str)], args: &[&str]) -> Run {
        let mut command = Command::new(assert_cmd::cargo::cargo_bin!("obr"));
        command.current_dir(&self.root);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("BD_")
                || name.starts_with("BEADS_")
                || name.starts_with("BR_")
                || name.starts_with("OBR_")
                || name == "TOON_DEFAULT_FORMAT"
                || name == "TOON_STATS"
            {
                command.env_remove(&key);
            }
        }
        command
            .env("HOME", &self.root)
            .env("NO_COLOR", "1")
            .env("RUST_LOG", "error");
        for (key, value) in env {
            command.env(key, value);
        }
        Run {
            output: command.args(args).output().expect("spawn obr"),
        }
    }

    /// Build a workspace, then rewrite it into its pre-rename shape.
    ///
    /// `dir_name` selects which legacy workspace directory to produce
    /// (`.beads` or `_beads`). The database is renamed to `beads.db` and the
    /// recorded database name is dropped from `metadata.json`, which is exactly
    /// the on-disk state a workspace created before the rename has.
    ///
    /// The export is moved back IN-DIR as `<dir>/issues.org` and the tracked
    /// surface `obr init` seeded at the root is removed. That is the D-SURFACE
    /// legacy layout: an export inside the workspace directory and no
    /// `PLAN.org` anywhere — which is what makes these rows exercise the
    /// no-forced-migration path instead of quietly resolving to a surface that
    /// only exists because the fixture's own `init` created it.
    fn make_legacy_workspace(&self, dir_name: &str) -> PathBuf {
        let run = self.run(&["init"]);
        assert!(run.success(), "init failed: {}", run.stderr());
        let current = self.path(".obr");
        let legacy = self.path(dir_name);
        fs::rename(current, &legacy).expect("rename workspace dir");
        fs::rename(legacy.join("obr.db"), legacy.join("beads.db")).expect("rename db");
        for suffix in ["-wal", "-shm"] {
            let from = legacy.join(format!("obr.db{suffix}"));
            if from.exists() {
                fs::rename(&from, legacy.join(format!("beads.db{suffix}")))
                    .expect("rename sidecar");
            }
        }
        fs::rename(self.path("PLAN.org"), legacy.join("issues.org")).expect("move export in-dir");
        fs::write(
            legacy.join("metadata.json"),
            "{\n  \"jsonl_export\": \"issues.org\"\n}",
        )
        .expect("rewrite metadata");
        legacy
    }
}

struct Run {
    output: Output,
}

impl Run {
    fn success(&self) -> bool {
        self.output.status.success()
    }

    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }

    /// Deprecation warnings mentioning `token`, in either rendering.
    ///
    /// Plain runs emit `Warning: …` lines; `--json`/`--toon` runs emit a
    /// one-line `{"warning":{…}}` envelope instead. Both are stderr-only, so
    /// counting them together is what "warned once" means regardless of mode.
    fn warnings_mentioning(&self, token: &str) -> Vec<String> {
        self.stderr()
            .lines()
            .filter(|line| {
                if let Ok(value) = serde_json::from_str::<Value>(line) {
                    return value
                        .get("warning")
                        .is_some_and(|warning| warning.to_string().contains(token));
                }
                line.starts_with("Warning: ") && line.contains(token)
            })
            .map(str::to_owned)
            .collect()
    }

    /// Assert exactly one deprecation warning named `token` was emitted.
    fn assert_warned_once(&self, token: &str) {
        let warnings = self.warnings_mentioning(token);
        assert_eq!(
            warnings.len(),
            1,
            "expected exactly one deprecation warning for {token}, got {warnings:?}\n\
             full stderr:\n{}",
            self.stderr()
        );
    }

    fn assert_no_warning(&self, token: &str) {
        let warnings = self.warnings_mentioning(token);
        assert!(
            warnings.is_empty(),
            "unexpected deprecation warning for {token}: {warnings:?}"
        );
    }

    /// The `{"warning":{…}}` envelopes emitted on stderr under `--json`.
    fn json_warnings(&self) -> Vec<Value> {
        self.stderr()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|value| value.get("warning").is_some())
            .collect()
    }

    fn stdout_json(&self) -> Value {
        serde_json::from_str(self.stdout().trim())
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {:?}", self.stdout()))
    }
}

// ---------------------------------------------------------------------------
// A1/A2 — workspace directory
// ---------------------------------------------------------------------------

#[test]
fn a1_legacy_beads_workspace_is_opened_with_one_warning() {
    let sandbox = Sandbox::new();
    let legacy = sandbox.make_legacy_workspace(".beads");

    let run = sandbox.run(&["info", "--json"]);
    assert!(run.success(), "info failed: {}", run.stderr());
    run.assert_warned_once(".beads is a legacy name");
    assert_eq!(
        run.stdout_json()["obr_dir"].as_str().map(PathBuf::from),
        Some(legacy)
    );
}

#[test]
fn a1_fresh_init_creates_only_the_new_workspace_dir() {
    let sandbox = Sandbox::new();
    let run = sandbox.run(&["init"]);
    assert!(run.success(), "init failed: {}", run.stderr());

    assert!(sandbox.path(".obr/obr.db").is_file());
    assert!(!sandbox.path(".beads").exists());
    assert!(!sandbox.path(".obr/beads.db").exists());
    assert!(!sandbox.path(".obr/.br_history").exists());
    assert!(!sandbox.path(".obr/.br_recovery").exists());

    let metadata = fs::read_to_string(sandbox.path(".obr/metadata.json")).expect("metadata");
    assert!(metadata.contains("\"obr.db\""), "metadata: {metadata}");

    let config = fs::read_to_string(sandbox.path(".obr/config.yaml")).expect("config");
    assert!(
        config.starts_with("# obr project configuration"),
        "config: {config}"
    );

    // D-SURFACE: a FRESH workspace tracks `PLAN.org` at the root and ignores
    // `.obr/` wholesale, so the inner ignore file no longer enumerates
    // per-artifact globs — it is a comment plus a bare `*`.
    let gitignore = fs::read_to_string(sandbox.path(".obr/.gitignore")).expect("gitignore");
    assert_eq!(
        gitignore
            .lines()
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>(),
        vec!["*"],
        "inner .gitignore should ignore the whole workspace: {gitignore}"
    );

    let surface = sandbox.path("PLAN.org");
    assert!(surface.is_file(), "init should seed the tracked surface");
    assert_eq!(fs::read_to_string(&surface).expect("surface"), "");
    assert!(
        !sandbox.path(".obr/issues.org").exists(),
        ".obr/ is cache only and must not hold an export"
    );
    assert!(!sandbox.path("doc").exists(), "init must never create doc/");
}

#[test]
fn a1_init_does_not_create_a_second_workspace_beside_a_legacy_one() {
    let sandbox = Sandbox::new();
    sandbox.make_legacy_workspace(".beads");

    // Re-running init must resolve to the existing legacy workspace, not carve
    // out a fresh `.obr` beside it and split the project in two.
    let run = sandbox.run(&["init"]);
    assert!(
        !sandbox.path(".obr").exists(),
        "init created .obr inside a legacy workspace: {}",
        run.stderr()
    );
    assert!(sandbox.path(".beads/beads.db").is_file());
}

#[test]
fn a2_legacy_underscore_workspace_is_opened_with_one_warning() {
    let sandbox = Sandbox::new();
    let legacy = sandbox.make_legacy_workspace("_beads");

    let run = sandbox.run(&["info", "--json"]);
    assert!(run.success(), "info failed: {}", run.stderr());
    run.assert_warned_once("_beads is a legacy name");
    assert_eq!(
        run.stdout_json()["obr_dir"].as_str().map(PathBuf::from),
        Some(legacy)
    );
}

#[test]
fn a1_new_workspace_dir_wins_over_a_stale_legacy_one() {
    let sandbox = Sandbox::new();
    let run = sandbox.run(&["init"]);
    assert!(run.success(), "init failed: {}", run.stderr());
    fs::create_dir(sandbox.path(".beads")).expect("stale legacy dir");

    let run = sandbox.run(&["info", "--json"]);
    assert!(run.success(), "info failed: {}", run.stderr());
    run.assert_no_warning(".beads is a legacy name");
    assert_eq!(
        run.stdout_json()["obr_dir"].as_str().map(PathBuf::from),
        Some(sandbox.path(".obr"))
    );
}

// ---------------------------------------------------------------------------
// A3 — database filename
// ---------------------------------------------------------------------------

#[test]
fn a3_legacy_database_is_opened_in_place_and_never_renamed() {
    let sandbox = Sandbox::new();
    let legacy = sandbox.make_legacy_workspace(".beads");

    let run = sandbox.run(&["info", "--json"]);
    assert!(run.success(), "info failed: {}", run.stderr());
    run.assert_warned_once("beads.db is a legacy name");
    assert_eq!(
        run.stdout_json()["database_path"]
            .as_str()
            .map(PathBuf::from),
        Some(legacy.join("beads.db"))
    );

    // The file stays where the user put it. Nothing is moved or copied.
    assert!(legacy.join("beads.db").is_file());
    assert!(!legacy.join("obr.db").exists());
}

#[test]
fn a3_fallback_yields_to_an_explicit_metadata_database_name() {
    let sandbox = Sandbox::new();
    let legacy = sandbox.make_legacy_workspace(".beads");
    fs::write(
        legacy.join("metadata.json"),
        "{\n  \"database\": \"obr.db\",\n  \"jsonl_export\": \"issues.org\"\n}",
    )
    .expect("rewrite metadata");

    // metadata names obr.db explicitly, so the missing file is a real error
    // rather than an invitation to reach for beads.db.
    let run = sandbox.run(&["info", "--json"]);
    run.assert_no_warning("beads.db is a legacy name");
    assert!(
        !run.success() || run.stdout_json()["database_path"] != Value::Null,
        "unexpected outcome: {}",
        run.stderr()
    );
}

#[test]
fn a3_new_database_wins_when_both_names_are_present() {
    let sandbox = Sandbox::new();
    let run = sandbox.run(&["init"]);
    assert!(run.success(), "init failed: {}", run.stderr());
    fs::write(sandbox.path(".obr/beads.db"), b"not a database").expect("decoy");

    let run = sandbox.run(&["info", "--json"]);
    assert!(run.success(), "info failed: {}", run.stderr());
    run.assert_no_warning("beads.db is a legacy name");
    assert_eq!(
        run.stdout_json()["database_path"]
            .as_str()
            .map(PathBuf::from),
        Some(sandbox.path(".obr/obr.db"))
    );
}

// ---------------------------------------------------------------------------
// A4 — legacy flat-file export name
// ---------------------------------------------------------------------------

#[test]
fn a4_legacy_beads_jsonl_is_the_resolved_source_with_one_warning() {
    let sandbox = Sandbox::new();
    let legacy = sandbox.make_legacy_workspace(".beads");
    fs::remove_file(legacy.join("issues.org")).expect("drop org export");
    fs::write(legacy.join("beads.jsonl"), "").expect("legacy export");

    let run = sandbox.run(&["info", "--json"]);
    assert!(run.success(), "info failed: {}", run.stderr());
    run.assert_warned_once("beads.jsonl is a legacy name");
    assert_eq!(
        run.stdout_json()["jsonl_path"].as_str().map(PathBuf::from),
        Some(legacy.join("beads.jsonl"))
    );
}

#[test]
fn a4_flush_writes_the_org_export_and_leaves_beads_jsonl_alone() {
    let sandbox = Sandbox::new();
    let legacy = sandbox.make_legacy_workspace(".beads");
    fs::write(legacy.join("beads.jsonl"), "").expect("legacy export");

    let run = sandbox.run(&["create", "--title=flush target", "--json"]);
    assert!(run.success(), "create failed: {}", run.stderr());
    let run = sandbox.run(&["sync", "--flush-only"]);
    assert!(run.success(), "flush failed: {}", run.stderr());

    // issues.org outranks beads.jsonl in the discovery chain, so the export
    // lands there and the pre-rename file is never written.
    let org = fs::read_to_string(legacy.join("issues.org")).expect("org export");
    assert!(org.contains("flush target"), "org export: {org}");
    assert_eq!(
        fs::read_to_string(legacy.join("beads.jsonl")).expect("legacy export"),
        ""
    );
    // D-SURFACE does NOT migrate a legacy workspace. The in-dir export stays
    // authoritative and no tracked surface is conjured beside it; relocating an
    // existing project's export would be a silent, unrequested git change.
    assert!(
        !sandbox.path("PLAN.org").exists(),
        "flush created a surface in a legacy workspace"
    );
}

// ---------------------------------------------------------------------------
// A6 — merge artifacts
// ---------------------------------------------------------------------------

#[test]
fn a6_both_merge_artifact_generations_are_excluded_from_export_discovery() {
    use obr::config::is_excluded_jsonl;

    for name in [
        "merge.base.jsonl",
        "merge.left.jsonl",
        "merge.right.jsonl",
        "beads.base.jsonl",
        "beads.left.jsonl",
        "beads.right.jsonl",
    ] {
        assert!(is_excluded_jsonl(name), "{name} must never be an export");
    }
}

#[test]
fn a6_merge_base_resolves_to_the_new_name_and_still_finds_a_legacy_one() {
    use obr::config::{MERGE_BASE_JSONL_FILENAME, merge_base_jsonl_path};

    let sandbox = Sandbox::new();
    let dir = sandbox.path("workspace");
    fs::create_dir(&dir).expect("workspace");

    // Nothing on disk: new merges write the current name.
    assert_eq!(
        merge_base_jsonl_path(&dir),
        dir.join(MERGE_BASE_JSONL_FILENAME)
    );

    // An interrupted pre-rename merge is still consumed.
    fs::write(dir.join("beads.base.jsonl"), "").expect("legacy anchor");
    assert_eq!(merge_base_jsonl_path(&dir), dir.join("beads.base.jsonl"));

    // Once the current name exists it wins outright.
    fs::write(dir.join(MERGE_BASE_JSONL_FILENAME), "").expect("anchor");
    assert_eq!(
        merge_base_jsonl_path(&dir),
        dir.join(MERGE_BASE_JSONL_FILENAME)
    );
}

// ---------------------------------------------------------------------------
// A7 — history directory
// ---------------------------------------------------------------------------

#[test]
fn a7_legacy_history_backups_stay_listable() {
    let sandbox = Sandbox::new();
    let legacy = sandbox.make_legacy_workspace(".beads");
    let history = legacy.join(".br_history");
    fs::create_dir(&history).expect("legacy history dir");
    fs::write(history.join("issues.20250101_000000.org"), "old backup")
        .expect("legacy backup file");

    let run = sandbox.run(&["history", "list"]);
    assert!(run.success(), "history list failed: {}", run.stderr());
    assert!(
        run.stdout().contains("issues.20250101_000000.org"),
        "legacy backup not listed: {}",
        run.stdout()
    );
    run.assert_warned_once(".br_history is a legacy name");
}

#[test]
fn a7_next_backup_migrates_the_legacy_history_dir_without_duplicating() {
    let sandbox = Sandbox::new();
    let legacy = sandbox.make_legacy_workspace(".beads");

    // Produce a genuine backup, then move it under the pre-rename directory
    // name. Seeding a hand-written file instead would date it beyond the
    // retention window and rotation would prune it before we could look.
    let run = sandbox.run(&["create", "--title=seeds a backup", "--json"]);
    assert!(run.success(), "create failed: {}", run.stderr());
    let history = legacy.join("history");
    let legacy_history = legacy.join(".br_history");
    fs::rename(&history, &legacy_history).expect("stage a pre-rename history dir");
    let seeded: Vec<PathBuf> = fs::read_dir(&legacy_history)
        .expect("read legacy history")
        .map(|entry| entry.expect("entry").path())
        .collect();
    assert!(!seeded.is_empty(), "no backup was written to seed with");

    let run = sandbox.run(&["create", "--title=triggers migration", "--json"]);
    assert!(run.success(), "create failed: {}", run.stderr());

    assert!(history.is_dir(), "history/ was not created");
    assert!(
        !legacy_history.exists(),
        "legacy history dir survived migration, backups would be duplicated"
    );
    for path in seeded {
        let name = path.file_name().expect("file name");
        assert!(
            history.join(name).exists(),
            "backup {name:?} was lost during migration"
        );
    }
}

// ---------------------------------------------------------------------------
// A8 — recovery directory
// ---------------------------------------------------------------------------

#[test]
fn a8_recovery_dir_prefers_the_new_name_and_adopts_an_existing_legacy_one() {
    let sandbox = Sandbox::new();
    let run = sandbox.run(&["init"]);
    assert!(run.success(), "init failed: {}", run.stderr());

    // A fresh workspace never grows a `.br_recovery/`.
    assert!(!sandbox.path(".obr/.br_recovery").exists());

    // A workspace that already has one keeps using it, so backups written
    // before the rename stay beside the ones written after.
    fs::create_dir(sandbox.path(".obr/.br_recovery")).expect("legacy recovery dir");
    let run = sandbox.run(&["doctor", "--json"]);
    run.assert_warned_once(".br_recovery is a legacy name");
}

// ---------------------------------------------------------------------------
// A13 — foreign snapshot debris
// ---------------------------------------------------------------------------

#[test]
fn a13_both_snapshot_debris_generations_are_reported() {
    let sandbox = Sandbox::new();
    let run = sandbox.run(&["init"]);
    assert!(run.success(), "init failed: {}", run.stderr());
    for debris in [".beads_snapshot", ".obr_snapshot"] {
        fs::create_dir(sandbox.path(&format!(".obr/{debris}"))).expect("debris dir");
    }

    let run = sandbox.run(&["doctor", "--json"]);
    let report = run.stdout();
    for debris in [".beads_snapshot", ".obr_snapshot"] {
        assert!(report.contains(debris), "{debris} not reported: {report}");
    }
    // obr's own state directories must never be mistaken for foreign debris.
    for own in ["\"recovery\"", "\"history\""] {
        assert!(
            !report.contains(own),
            "own state flagged as debris: {report}"
        );
    }
}

// ---------------------------------------------------------------------------
// A17 — agent instruction markers
// ---------------------------------------------------------------------------

#[test]
fn a17_agents_update_replaces_a_legacy_marker_block_in_place() {
    let sandbox = Sandbox::new();
    let run = sandbox.run(&["init"]);
    assert!(run.success(), "init failed: {}", run.stderr());

    let agents = sandbox.path("AGENTS.md");
    fs::write(
        &agents,
        "# Project\n\nSome prose.\n\n\
         <!-- br-agent-instructions-v1 -->\nold blurb body\n<!-- end-br-agent-instructions -->\n",
    )
    .expect("seed AGENTS.md");

    let run = sandbox.run(&["agents", "--update", "--force"]);
    assert!(run.success(), "agents --update failed: {}", run.stderr());

    let content = fs::read_to_string(&agents).expect("AGENTS.md");
    assert_eq!(
        content.matches("agent-instructions-v").count(),
        1,
        "update appended a second block instead of replacing:\n{content}"
    );
    assert!(content.contains("<!-- obr-agent-instructions-v1 -->"));
    assert!(!content.contains("<!-- br-agent-instructions-v1 -->"));
    assert!(!content.contains("old blurb body"));
    assert!(content.contains("Some prose."), "surrounding prose lost");
}

// ---------------------------------------------------------------------------
// M1 — versioned schema identifiers
// ---------------------------------------------------------------------------

#[test]
fn m1_read_back_schema_ids_accept_the_pre_rename_spelling() {
    use obr::legacy_compat::schema_id_accepted;

    for current in [
        "obr.doctor.schema_migration.applied.v1",
        "obr.doctor.schema_migration.prepared.v1",
        "obr.doctor.schema_migration.undo.v1",
        "obr.doctor.db_snapshot.v1",
    ] {
        let legacy = current.replacen("obr.", "br.", 1);
        assert!(schema_id_accepted(current, current), "{current}");
        assert!(schema_id_accepted(&legacy, current), "{legacy}");
        assert!(
            !schema_id_accepted("obr.doctor.something_else.v1", current),
            "{current} accepted an unrelated schema"
        );
    }
}

#[test]
fn m1_new_artifacts_are_written_with_the_obr_schema_id() {
    let sandbox = Sandbox::new();
    let run = sandbox.run(&["init"]);
    assert!(run.success(), "init failed: {}", run.stderr());

    let run = sandbox.run(&["capabilities", "--json"]);
    assert!(run.success(), "capabilities failed: {}", run.stderr());
    assert_eq!(run.stdout_json()["contract_version"], "obr.capabilities.v1");

    let run = sandbox.run(&["doctor", "capabilities", "--format", "json"]);
    assert!(
        run.success(),
        "doctor capabilities failed: {}",
        run.stderr()
    );
    assert_eq!(
        run.stdout_json()["schema_version"],
        "obr.doctor.capabilities.v1"
    );
}

// ---------------------------------------------------------------------------
// Warning envelope shape
// ---------------------------------------------------------------------------

#[test]
fn warnings_are_json_safe_and_never_touch_stdout() {
    let sandbox = Sandbox::new();
    sandbox.make_legacy_workspace(".beads");

    let run = sandbox.run(&["info", "--json"]);
    assert!(run.success(), "info failed: {}", run.stderr());

    // stdout stays parseable JSON even though warnings fired.
    let payload = run.stdout_json();
    assert!(payload.get("obr_dir").is_some());
    assert!(
        !run.stdout().contains("Warning:"),
        "a warning leaked onto stdout: {}",
        run.stdout()
    );

    let envelopes = run.json_warnings();
    assert!(
        !envelopes.is_empty(),
        "no JSON warning envelope on stderr: {}",
        run.stderr()
    );
    // Every envelope carries a code and a message, whatever its class. A legacy
    // workspace emits both classes: the rename deprecations, and the D-SURFACE
    // advisory that its in-dir export is being kept rather than migrated.
    let mut deprecations = 0;
    for envelope in &envelopes {
        let warning = &envelope["warning"];
        for field in ["code", "message"] {
            assert!(
                warning[field].is_string(),
                "envelope missing {field}: {envelope}"
            );
        }
        if warning["code"] == "DEPRECATED_NAME" {
            deprecations += 1;
            for field in ["legacy", "replacement"] {
                assert!(
                    warning[field].is_string(),
                    "deprecation envelope missing {field}: {envelope}"
                );
            }
        } else {
            assert_eq!(warning["code"], "WORKSPACE_ADVISORY", "{envelope}");
            assert!(
                warning["key"].is_string(),
                "advisory missing key: {envelope}"
            );
        }
    }
    assert!(
        deprecations > 0,
        "a legacy workspace emitted no deprecation envelope: {:?}",
        envelopes
    );
}

#[test]
fn warnings_are_suppressed_under_quiet() {
    let sandbox = Sandbox::new();
    sandbox.make_legacy_workspace(".beads");

    let run = sandbox.run(&["--quiet", "list"]);
    assert!(
        run.warnings_mentioning(".beads").is_empty() && run.json_warnings().is_empty(),
        "quiet run emitted a deprecation warning: {}",
        run.stderr()
    );
}

#[test]
fn each_legacy_name_warns_at_most_once_per_process() {
    let sandbox = Sandbox::new();
    sandbox.make_legacy_workspace(".beads");

    // `doctor` re-resolves the workspace and database repeatedly; a per-lookup
    // warning would show up here as a flood.
    let run = sandbox.run(&["doctor"]);
    for token in [".beads is a legacy name", "beads.db is a legacy name"] {
        let warnings = run.warnings_mentioning(token);
        assert!(
            warnings.len() <= 1,
            "{token} warned {} times: {warnings:?}",
            warnings.len()
        );
    }
}
