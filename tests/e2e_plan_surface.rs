//! P6-06 (D-SURFACE): the tracked human surface is `PLAN.org`.
//!
//! `.obr/` holds only per-machine cache and is ignored wholesale; the one
//! artifact this workspace puts in version control is `PLAN.org`, under the
//! first of `doc/`, `docs/` the project already has, else at the root.
//!
//! Every scenario here is unconditional. A test that branched on which file
//! happens to exist would pass against either resolution and prove nothing, so
//! each asserts one layout exactly. Temp roots are canonicalized because macOS
//! hands out `/var/folders/...`, a symlink into `/private/var`, and the binary
//! reports the resolved path.

mod common;

use common::cli::{ObrWorkspace, run_obr};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn canonical(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn root_of(workspace: &ObrWorkspace) -> PathBuf {
    canonical(&workspace.root)
}

/// The `jsonl_path` the binary reports for this workspace.
fn reported_export(workspace: &ObrWorkspace) -> PathBuf {
    let info = run_obr(workspace, ["info", "--json"], "info");
    assert!(info.status.success(), "info failed: {}", info.stderr);
    let start = info.stdout.find('{').expect("info json");
    let value: Value = serde_json::from_str(&info.stdout[start..]).expect("parse info json");
    PathBuf::from(
        value["jsonl_path"]
            .as_str()
            .expect("info reported no jsonl_path"),
    )
}

fn init(workspace: &ObrWorkspace, prefix: &str) {
    let run = run_obr(workspace, ["init", "--prefix", prefix], "init");
    assert!(run.status.success(), "init failed: {}", run.stderr);
}

fn create(workspace: &ObrWorkspace, title: &str) {
    let run = run_obr(
        workspace,
        ["create", title, "--type", "task", "--priority", "2"],
        "create",
    );
    assert!(run.status.success(), "create failed: {}", run.stderr);
}

fn flush(workspace: &ObrWorkspace) {
    let run = run_obr(workspace, ["sync", "--flush-only"], "flush");
    assert!(run.status.success(), "flush failed: {}", run.stderr);
}

// ---------------------------------------------------------------------------
// 1. Fresh init without doc/ — the surface lands at the workspace root.
// ---------------------------------------------------------------------------

#[test]
fn fresh_init_seeds_the_surface_at_the_root() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "sfc");
    let root = root_of(&workspace);

    let surface = root.join("PLAN.org");
    assert!(surface.is_file(), "init did not seed {}", surface.display());
    assert_eq!(fs::read_to_string(&surface).expect("surface"), "");

    // `.obr/` is cache only: no export inside it, and it ignores itself.
    assert!(!root.join(".obr/issues.org").exists());
    assert!(!root.join(".obr/issues.jsonl").exists());
    assert!(!root.join(".obr/PLAN.org").exists());
    let gitignore = fs::read_to_string(root.join(".obr/.gitignore")).expect("inner gitignore");
    assert_eq!(
        gitignore
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .collect::<Vec<_>>(),
        vec!["*"],
        ".obr/.gitignore must ignore the directory wholesale: {gitignore}"
    );

    // obr never creates doc/ — it only uses one the project already has.
    assert!(!root.join("doc").exists(), "init created doc/");

    assert_eq!(reported_export(&workspace), surface);
}

// ---------------------------------------------------------------------------
// 2. Fresh init WITH doc/ — the surface lands under it.
// ---------------------------------------------------------------------------

#[test]
fn fresh_init_seeds_the_surface_under_an_existing_doc_dir() {
    let workspace = ObrWorkspace::new();
    fs::create_dir(workspace.root.join("doc")).expect("doc dir");
    init(&workspace, "sfc");
    let root = root_of(&workspace);

    let surface = root.join("doc/PLAN.org");
    assert!(surface.is_file(), "init did not seed {}", surface.display());
    assert!(
        !root.join("PLAN.org").exists(),
        "a root PLAN.org was seeded alongside doc/PLAN.org"
    );
    assert_eq!(reported_export(&workspace), surface);
}

/// `docs/` is the second probe, used only when the project has no `doc/`.
#[test]
fn fresh_init_seeds_the_surface_under_an_existing_docs_dir() {
    let workspace = ObrWorkspace::new();
    fs::create_dir(workspace.root.join("docs")).expect("docs dir");
    init(&workspace, "sfc");
    let root = root_of(&workspace);

    let surface = root.join("docs/PLAN.org");
    assert!(surface.is_file(), "init did not seed {}", surface.display());
    assert!(!root.join("PLAN.org").exists());
    assert!(!root.join("doc").exists(), "init created doc/");
    assert_eq!(reported_export(&workspace), surface);
}

/// With both present, `doc/` outranks `docs/` — the probe order is a
/// precedence rule, not a search for whichever happens to exist.
#[test]
fn doc_dir_outranks_docs_dir_when_both_exist() {
    let workspace = ObrWorkspace::new();
    fs::create_dir(workspace.root.join("doc")).expect("doc dir");
    fs::create_dir(workspace.root.join("docs")).expect("docs dir");
    init(&workspace, "sfc");
    let root = root_of(&workspace);

    assert!(root.join("doc/PLAN.org").is_file());
    assert!(
        !root.join("docs/PLAN.org").exists(),
        "init seeded the lower-priority docs/ location"
    );
    assert_eq!(reported_export(&workspace), root.join("doc/PLAN.org"));
}

// ---------------------------------------------------------------------------
// 2b. The write target is stable for the life of a workspace: an EXISTING
//     surface outranks the directory preference, always.
// ---------------------------------------------------------------------------

/// Creating `doc/` after the fact must not move a live surface. The
/// directory preference decides where a surface goes when there is none; it
/// never relocates one that exists, and it says nothing, because nothing
/// happened.
#[test]
fn creating_doc_dir_later_does_not_move_an_existing_root_surface() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "stb");
    let root = root_of(&workspace);
    create(&workspace, "first issue");
    flush(&workspace);
    assert!(root.join("PLAN.org").is_file());

    fs::create_dir(workspace.root.join("doc")).expect("doc dir");
    create(&workspace, "second issue");
    flush(&workspace);

    let surface = fs::read_to_string(root.join("PLAN.org")).expect("root surface");
    assert!(
        surface.contains("first issue") && surface.contains("second issue"),
        "the root surface stopped receiving writes: {surface}"
    );
    assert!(
        !root.join("doc/PLAN.org").exists(),
        "a doc/ created after the fact stole the write target"
    );
    assert_eq!(reported_export(&workspace), root.join("PLAN.org"));
}

/// Same rule for the `docs/` slot, and for a `doc/` that appears next to a
/// live `docs/PLAN.org` — the higher-priority *directory* does not outrank a
/// surface that already exists.
#[test]
fn creating_doc_dir_later_does_not_move_an_existing_docs_surface() {
    let workspace = ObrWorkspace::new();
    fs::create_dir(workspace.root.join("docs")).expect("docs dir");
    init(&workspace, "stb");
    let root = root_of(&workspace);
    create(&workspace, "first issue");
    flush(&workspace);
    assert!(root.join("docs/PLAN.org").is_file());

    fs::create_dir(workspace.root.join("doc")).expect("doc dir");
    create(&workspace, "second issue");
    flush(&workspace);

    let surface = fs::read_to_string(root.join("docs/PLAN.org")).expect("docs surface");
    assert!(
        surface.contains("first issue") && surface.contains("second issue"),
        "the docs/ surface stopped receiving writes: {surface}"
    );
    assert!(!root.join("doc/PLAN.org").exists());
    assert_eq!(reported_export(&workspace), root.join("docs/PLAN.org"));
}

/// The fresh-clone case, and the reason `init` resolves instead of computing:
/// a checkout with a tracked `PLAN.org` at the root AND a `doc/` directory
/// used to have an empty `doc/PLAN.org` seeded over it, which outranks the
/// tracked file — `obr list` then reported an empty tracker while every real
/// issue sat in the shadowed file.
#[test]
fn init_on_a_clone_adopts_the_tracked_surface_instead_of_shadowing_it() {
    let workspace = ObrWorkspace::new();
    fs::create_dir(workspace.root.join("doc")).expect("doc dir");
    fs::write(
        workspace.root.join("PLAN.org"),
        "#+TITLE: Obr Issues\n\
         #+SEQ_TODO: TODO DOING DRAFT WAIT DEFER NOTE | DONE CANCELED\n\
         #+ISSUE_PREFIX: clone\n\n\
         * TODO [#C] tracked issue\n\
         :PROPERTIES:\n\
         :ID:       clone-1\n\
         :CREATED_AT: 2026-01-01T00:00:00+00:00\n\
         :UPDATED_AT: 2026-01-01T00:00:00+00:00\n\
         :END:\n\n",
    )
    .expect("tracked surface");

    init(&workspace, "clone");
    let root = root_of(&workspace);

    assert!(
        !root.join("doc/PLAN.org").exists(),
        "init seeded an empty doc/PLAN.org over the tracked root surface"
    );
    assert_eq!(reported_export(&workspace), root.join("PLAN.org"));

    let list = run_obr(&workspace, ["list", "--json"], "list after clone init");
    assert!(list.status.success(), "list failed: {}", list.stderr);
    assert!(
        list.stdout.contains("tracked issue"),
        "the tracked issue is invisible after init: {}",
        list.stdout
    );
}

// ---------------------------------------------------------------------------
// 3. create → flush → import round-trip through the surface.
// ---------------------------------------------------------------------------

#[test]
fn flush_writes_the_issue_and_the_prefix_keyword_to_the_surface() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "sfc");
    create(&workspace, "surface round trip");
    flush(&workspace);

    let surface = root_of(&workspace).join("PLAN.org");
    let content = fs::read_to_string(&surface).expect("surface");
    assert!(
        content.contains("surface round trip"),
        "flush did not write the issue to the surface: {content}"
    );
    assert!(
        content.contains("#+ISSUE_PREFIX: sfc"),
        "flush did not record the prefix keyword: {content}"
    );

    // The surface is a real export: a --no-db reader works off it alone.
    let show = run_obr(&workspace, ["--no-db", "list", "--json"], "no-db list");
    assert!(
        show.status.success(),
        "--no-db list failed: {}",
        show.stderr
    );
    assert!(
        show.stdout.contains("surface round trip"),
        "--no-db list did not read the surface: {}",
        show.stdout
    );

    // Re-import into a rebuilt database recovers the issue.
    let import = run_obr(
        &workspace,
        ["sync", "--import-only", "--rebuild"],
        "reimport",
    );
    assert!(import.status.success(), "import failed: {}", import.stderr);
    let list = run_obr(&workspace, ["list", "--json"], "list");
    assert!(list.status.success(), "list failed: {}", list.stderr);
    assert!(
        list.stdout.contains("surface round trip"),
        "round-trip lost the issue: {}",
        list.stdout
    );
}

// ---------------------------------------------------------------------------
// 4. A legacy workspace keeps its in-dir export — no forced migration.
// ---------------------------------------------------------------------------

#[test]
fn legacy_in_dir_export_is_kept_and_warns_once() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "lgc");
    let root = root_of(&workspace);

    // Rewind to the pre-D-SURFACE layout: the export sits inside `.obr/` and
    // no tracked surface exists anywhere.
    fs::rename(root.join("PLAN.org"), root.join(".obr/issues.org")).expect("move export in-dir");

    assert_eq!(reported_export(&workspace), root.join(".obr/issues.org"));

    create(&workspace, "legacy target");
    flush(&workspace);

    let in_dir = fs::read_to_string(root.join(".obr/issues.org")).expect("in-dir export");
    assert!(
        in_dir.contains("legacy target"),
        "flush abandoned the in-dir export: {in_dir}"
    );
    assert!(
        !root.join("PLAN.org").exists(),
        "flush relocated a legacy workspace's export without being asked"
    );

    // The advisory fires exactly once per process, on stderr, in JSON form.
    let info = run_obr(&workspace, ["info", "--json"], "info advisory");
    let advisories: Vec<&str> = info
        .stderr
        .lines()
        .filter(|line| line.contains("pre-D-SURFACE in-dir export"))
        .collect();
    assert_eq!(
        advisories.len(),
        1,
        "expected exactly one in-dir advisory, got {advisories:?}"
    );
    assert!(
        !info.stdout.contains("pre-D-SURFACE"),
        "advisory leaked onto stdout: {}",
        info.stdout
    );
}

// ---------------------------------------------------------------------------
// 5. Pinned metadata still wins — Class A workspaces are untouched.
// ---------------------------------------------------------------------------

#[test]
fn pinned_metadata_export_beats_the_surface_rule() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "pin");
    let root = root_of(&workspace);
    common::cli::pin_jsonl(&root.join(".obr"));

    let pinned = root.join(".obr/issues.jsonl");
    assert_eq!(reported_export(&workspace), pinned);

    create(&workspace, "pinned target");
    flush(&workspace);

    let content = fs::read_to_string(&pinned).expect("pinned export");
    assert!(
        content.contains("pinned target"),
        "flush ignored the pinned export: {content}"
    );
    assert!(
        !root.join("PLAN.org").exists(),
        "an explicit metadata override must not grow a surface beside it"
    );
}

// ---------------------------------------------------------------------------
// 6. Shadowing: doc/PLAN.org wins over a root PLAN.org, and says so.
// ---------------------------------------------------------------------------

#[test]
fn doc_surface_shadows_the_lower_priority_surfaces_and_warns() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "shd");
    let root = root_of(&workspace);

    // The root surface already exists from init; adding the subdirectory
    // copies after the fact is exactly how a project acquires more than one.
    for subdir in ["doc", "docs"] {
        fs::create_dir(root.join(subdir)).expect("surface dir");
        fs::write(root.join(subdir).join("PLAN.org"), "").expect("surface");
    }

    let shadowed = root.join("PLAN.org");
    let winner = root.join("doc/PLAN.org");
    assert_eq!(reported_export(&workspace), winner);

    create(&workspace, "shadow target");
    flush(&workspace);

    let content = fs::read_to_string(&winner).expect("doc surface");
    assert!(
        content.contains("shadow target"),
        "flush did not write the winning surface: {content}"
    );
    // Never merged: every shadowed file is left exactly as it was.
    for path in [&shadowed, &root.join("docs/PLAN.org")] {
        assert_eq!(
            fs::read_to_string(path).expect("shadowed surface"),
            "",
            "{} was written to despite being shadowed",
            path.display()
        );
    }

    let info = run_obr(&workspace, ["info", "--json"], "info shadow");
    assert!(info.status.success(), "info failed: {}", info.stderr);
    let warnings: Vec<&str> = info
        .stderr
        .lines()
        .filter(|line| line.contains("is shadowed by"))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "expected exactly one shadowing warning, got {warnings:?}"
    );
    // One warning, naming every file it is shadowing.
    for path in [&shadowed, &root.join("docs/PLAN.org")] {
        assert!(
            warnings[0].contains(&path.display().to_string()),
            "warning does not name {}: {}",
            path.display(),
            warnings[0]
        );
    }
    // JSON mode stays parseable with the warning in flight.
    let start = info.stdout.find('{').expect("info json");
    serde_json::from_str::<Value>(&info.stdout[start..]).expect("stdout is still valid JSON");
}

/// The prefix keyword is written whenever the prefix is KNOWN, not merely when
/// some issue happens to carry it. An empty workspace is precisely the corpus a
/// fresh clone would otherwise have to bootstrap from with nothing to go on.
#[test]
fn empty_workspace_flush_still_writes_the_prefix_keyword() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "mty");

    let flush = run_obr(
        &workspace,
        ["sync", "--flush-only", "--force"],
        "empty flush",
    );
    assert!(flush.status.success(), "flush failed: {}", flush.stderr);

    let content = fs::read_to_string(root_of(&workspace).join("PLAN.org")).expect("surface");
    assert!(
        content.contains("#+ISSUE_PREFIX: mty"),
        "an issue-free export dropped the prefix keyword: {content:?}"
    );
    // Nothing else got invented: the export really is issue-free.
    assert!(
        !content.contains("\n* "),
        "unexpected headings: {content:?}"
    );
}

// ---------------------------------------------------------------------------
// 7. Prefix bootstrap: identity survives a clone with no `.obr/`.
// ---------------------------------------------------------------------------

/// Per the 2026-08-07 ruling, `PLAN.org` is NOT a workspace marker — it is far
/// too generic a filename to justify auto-creating `.obr/` beside any file that
/// happens to bear it. The documented fresh-clone flow is `obr init && obr sync
/// --import-only --rebuild`, and `#+ISSUE_PREFIX` is authoritative over the
/// prefix `init` derives from the directory name, so identity survives.
#[test]
fn issue_prefix_keyword_survives_a_workspace_rebuild() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "xyz");
    create(&workspace, "prefix carrier");
    flush(&workspace);
    let root = root_of(&workspace);

    let surface_before = fs::read_to_string(root.join("PLAN.org")).expect("surface");
    assert!(surface_before.contains("#+ISSUE_PREFIX: xyz"));

    // What a fresh clone looks like: the tracked surface, and nothing else.
    fs::remove_dir_all(root.join(".obr")).expect("drop the cache dir");

    let reinit = run_obr(&workspace, ["init"], "bootstrap init");
    assert!(reinit.status.success(), "init failed: {}", reinit.stderr);
    let import = run_obr(
        &workspace,
        ["sync", "--import-only", "--rebuild"],
        "bootstrap import",
    );
    assert!(import.status.success(), "import failed: {}", import.stderr);

    // init derived a prefix from the temp directory name; the keyword overrides
    // it, so newly created issues keep the project's own identity.
    let created = run_obr(
        &workspace,
        ["create", "after bootstrap", "--type", "task", "--json"],
        "create after bootstrap",
    );
    assert!(
        created.status.success(),
        "create failed: {}",
        created.stderr
    );
    let start = created.stdout.find('{').expect("create json");
    let value: Value = serde_json::from_str(&created.stdout[start..]).expect("parse create json");
    let id = value["id"].as_str().expect("created id");
    assert!(
        id.starts_with("xyz-"),
        "bootstrap lost the #+ISSUE_PREFIX identity: {id}"
    );
}

// ---------------------------------------------------------------------------
// 8. Doctor: healthy on the new layout, and flags both inversions.
// ---------------------------------------------------------------------------

fn doctor_check(workspace: &ObrWorkspace, name: &str, label: &str) -> Value {
    let run = run_obr(workspace, ["doctor", "--json"], label);
    let start = run.stdout.find('{').expect("doctor json");
    let report: Value = serde_json::from_str(&run.stdout[start..]).expect("parse doctor json");
    report["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .find(|check| check["name"] == name)
        .unwrap_or_else(|| panic!("doctor did not run {name}: {}", run.stdout))
        .clone()
}

#[test]
fn doctor_is_healthy_on_the_new_layout() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "doc");
    create(&workspace, "doctor subject");
    flush(&workspace);
    // The root-gitignore check only runs when there IS a root .gitignore, so
    // spell out the layout this model recommends: cache hidden, surface not.
    fs::write(workspace.root.join(".gitignore"), "node_modules/\n.obr/\n").expect("root gitignore");

    for name in [
        "gitignore.obr_inner",
        "gitignore.obr_inner_present",
        "sync_jsonl_path",
        "metadata.json",
    ] {
        let check = doctor_check(&workspace, name, "doctor healthy");
        assert_eq!(check["status"], "ok", "{name} is not ok: {check}");
    }

    // The surface is in-allowlist, not an authorized external path.
    let check = doctor_check(&workspace, "sync_jsonl_path", "doctor surface");
    assert_eq!(check["details"]["surface"], true, "{check}");
    assert_eq!(check["details"]["external"], false, "{check}");
}

#[test]
fn doctor_flags_a_workspace_cache_that_git_would_track() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "doc");
    // The workspace stops ignoring itself: its per-machine cache is now visible
    // to git, which is the failure this check exists for.
    fs::write(
        workspace.root.join(".obr/.gitignore"),
        "# operator rules\n*.tmp\n",
    )
    .expect("weaken inner gitignore");

    let check = doctor_check(&workspace, "gitignore.obr_inner_present", "doctor tracked");
    assert_eq!(check["status"], "warn", "{check}");
    assert_eq!(check["details"]["kind"], "incomplete", "{check}");

    // A root .gitignore covering the whole directory satisfies it equally —
    // operators who keep one central ignore file are not forced into two.
    fs::write(workspace.root.join(".gitignore"), ".obr/\n").expect("root gitignore");
    let check = doctor_check(&workspace, "gitignore.obr_inner_present", "doctor covered");
    assert_eq!(check["status"], "ok", "{check}");
}

#[test]
fn doctor_flags_a_surface_that_git_would_ignore() {
    let workspace = ObrWorkspace::new();
    init(&workspace, "doc");
    fs::write(
        workspace.root.join(".gitignore"),
        "node_modules/\nPLAN.org\n",
    )
    .expect("root gitignore");

    let check = doctor_check(&workspace, "gitignore.obr_inner", "doctor ignored surface");
    assert_eq!(check["status"], "warn", "{check}");
    assert_eq!(
        check["details"]["offending_patterns"]
            .as_array()
            .expect("offending patterns"),
        &vec![Value::String("PLAN.org".to_string())],
        "{check}"
    );

    // `.obr/` being ignored is the DESIRED state under this model, so a root
    // .gitignore that hides only the cache must not be flagged.
    fs::write(workspace.root.join(".gitignore"), "node_modules/\n.obr/\n").expect("root gitignore");
    let check = doctor_check(&workspace, "gitignore.obr_inner", "doctor ignored cache");
    assert_eq!(check["status"], "ok", "{check}");
}
