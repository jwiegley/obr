use assert_cmd::Command;
use serde_json::Value;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use tempfile::TempDir;

const SMOKE_PRESERVED_ENV_KEYS: &[&str] = &[
    "OBR_DIR",
    "OBR_JSONL",
    "OBR_CACHE_DIR",
    "OBR_OUTPUT_FORMAT",
    "BEADS_DIR",
    "BEADS_JSONL",
    "BEADS_CACHE_DIR",
    "BR_OUTPUT_FORMAT",
    "TOON_DEFAULT_FORMAT",
    "TOON_STATS",
];

fn should_clear_inherited_obr_env(key: &OsStr) -> bool {
    let key = key.to_string_lossy();
    key.starts_with("OBR_")
        || key.starts_with("BD_")
        || key.starts_with("BEADS_")
        || matches!(
            key.as_ref(),
            "BR_DISABLE_READ_ONLY_FAST_OPEN"
                | "OBR_DISABLE_READ_ONLY_FAST_OPEN"
                | "BR_OUTPUT_FORMAT"
                | "TOON_DEFAULT_FORMAT"
                | "TOON_STATS"
        )
}

fn should_preserve_smoke_env(key: &OsStr) -> bool {
    let key = key.to_string_lossy();
    SMOKE_PRESERVED_ENV_KEYS.contains(&key.as_ref())
}

fn clear_inherited_obr_env(cmd: &mut Command) {
    clear_inherited_obr_env_except(cmd, &[]);
}

/// `$PATH` with every directory after the first that holds a `obr` executable
/// removed.
///
/// `obr doctor` reports `obr_path_dupes` when more than one `obr` is reachable on
/// `$PATH`, and since #292 any WARN flips `ok` to false and exits 1. A
/// developer who has both `~/.local/bin/obr` (install script) and
/// `~/.cargo/bin/obr` (`cargo install`) — the exact combination the README warns
/// about — would therefore fail every "healthy workspace" doctor test for
/// reasons that have nothing to do with the workspace under test.
///
/// Only later duplicates are dropped, so the first such directory survives and
/// siblings living beside it (notably the Go `bd` binary used by the
/// conformance suite) stay reachable. The dedicated
/// `tests/doctor_fixtures/multiple_obr_in_path` fixture builds its own `$PATH`
/// and so still exercises the detection deliberately.
pub fn deduplicated_obr_path() -> std::ffi::OsString {
    let Some(path) = std::env::var_os("PATH") else {
        return std::ffi::OsString::new();
    };
    let mut seen_obr_dir = false;
    let kept: Vec<std::path::PathBuf> = std::env::split_paths(&path)
        .filter(|dir| {
            let has_obr = dir.join("obr").is_file();
            if !has_obr {
                return true;
            }
            if seen_obr_dir {
                return false;
            }
            seen_obr_dir = true;
            true
        })
        .collect();
    std::env::join_paths(kept).unwrap_or(path)
}

fn clear_inherited_obr_env_except(cmd: &mut Command, preserve: &[&str]) {
    for (key, _) in std::env::vars_os() {
        let key_str = key.to_string_lossy();
        let should_preserve = preserve.contains(&key_str.as_ref());
        if should_clear_inherited_obr_env(&key) && !should_preserve {
            cmd.env_remove(&key);
        }
    }
}

#[derive(Debug)]
pub struct ObrRun {
    pub stdout: String,
    pub stderr: String,
    pub status: std::process::ExitStatus,
    pub duration: Duration,
    pub log_path: PathBuf,
}

pub struct ObrWorkspace {
    pub temp_dir: TempDir,
    pub root: PathBuf,
    pub log_dir: PathBuf,
}

/// Every workspace directory name `obr` will discover, current and pre-rename.
///
/// Discovery accepts all four, so a temp root is only safe when no ancestor
/// carries any of them.
pub const WORKSPACE_DIR_NAMES: [&str; 4] = [".obr", "_obr", ".beads", "_beads"];

/// The workspace directory name `obr init` creates.
pub const WORKSPACE_DIR: &str = ".obr";

/// The database filename `obr init` creates.
pub const DB_FILE: &str = "obr.db";

/// Whether any ancestor of `start` (inclusive) is an obr workspace.
pub fn is_inside_obr_workspace(start: &Path) -> bool {
    start.ancestors().any(|dir| {
        WORKSPACE_DIR_NAMES
            .iter()
            .any(|name| dir.join(name).is_dir())
    })
}

/// A temp root guaranteed not to sit inside an existing beads workspace.
///
/// `obr` resolves its workspace by walking every ancestor up to the filesystem
/// root with no `.git`-style boundary (`config::discover_obr_dir_candidate_with_env`).
/// A `TMPDIR` that lives inside a checkout therefore hands every workspace
/// created here the enclosing repo's `.obr/`, and the tests that assert on an
/// *uninitialized* workspace silently exercise the wrong one.
///
/// This is not hypothetical: rch points `TMPDIR` at `<repo>/.rch-tmp` for remote
/// builds, so under remote execution `obr list` in a "fresh" workspace returns
/// the obr repo's own issues and exits 0 where the test requires failure.
///
/// Prefer `TMPDIR`; fall back to a system temp root that is clean.
pub fn isolated_temp_root() -> PathBuf {
    let preferred = std::env::temp_dir();
    if !is_inside_obr_workspace(&preferred) {
        return preferred;
    }

    for fallback in ["/tmp", "/var/tmp"] {
        let path = PathBuf::from(fallback);
        if path.is_dir() && !is_inside_obr_workspace(&path) {
            return path;
        }
    }

    panic!(
        "no beads-free temp root available: TMPDIR ({}) is inside a obr workspace \
         and no system fallback is clean. Set TMPDIR to a directory outside any \
         .obr/ tree before running the test suite.",
        preferred.display()
    );
}

impl ObrWorkspace {
    pub fn new() -> Self {
        let temp_dir = TempDir::new_in(isolated_temp_root()).expect("temp dir");
        let root = temp_dir.path().to_path_buf();
        let log_dir = root.join("logs");
        fs::create_dir_all(&log_dir).expect("log dir");
        Self {
            temp_dir,
            root,
            log_dir,
        }
    }
}

/// The workspace's default export artifact: the tracked surface (D-SURFACE).
///
/// Class-B tests assert against this instead of a filename literal, which is
/// what let the artifact move out of `.obr/` and into the tracked project tree
/// without rewriting each of them.
#[allow(dead_code)]
pub fn export_path(workspace: &ObrWorkspace) -> PathBuf {
    obr::config::computed_surface_path(&workspace.root)
}

/// Pin an initialized workspace to the legacy JSONL export format (Class A
/// tests: they exercise JSONL-specific machinery, not the default path).
///
/// The default export is the Org surface; setting `metadata.json`'s
/// `jsonl_export` to `issues.jsonl` makes it an explicit override
/// (`resolve_jsonl_path` priority 2), which beats both discovery and the
/// surface rule. Every seed `init` may have written — the tracked surface and
/// the pre-D-SURFACE in-dir `issues.org` — is removed so discovery-order tests
/// and directory assertions see a pure JSONL workspace.
pub fn pin_jsonl(obr_dir: &Path) {
    let metadata_path = obr_dir.join("metadata.json");
    let text = fs::read_to_string(&metadata_path).expect("read metadata.json");
    let mut value: serde_json::Value = serde_json::from_str(&text).expect("parse metadata.json");
    value["jsonl_export"] = serde_json::Value::String("issues.jsonl".to_string());
    fs::write(
        &metadata_path,
        serde_json::to_string_pretty(&value).expect("serialize metadata.json"),
    )
    .expect("write metadata.json");

    let jsonl = obr_dir.join("issues.jsonl");
    if !jsonl.exists() {
        fs::write(&jsonl, b"").expect("seed issues.jsonl");
    }
    let mut seeds = vec![obr_dir.join("issues.org")];
    if let Some(root) = obr_dir.parent() {
        seeds.push(obr::config::computed_surface_path(root));
    }
    for seed in seeds {
        if seed.exists() {
            fs::remove_file(&seed).expect("remove export seed");
        }
    }
}

pub fn run_obr<I, S>(workspace: &ObrWorkspace, args: I, label: &str) -> ObrRun
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    // Reuse run_obr_with_env with empty env vars
    run_obr_with_env(
        workspace,
        args,
        std::iter::empty::<(String, String)>(),
        label,
    )
}

pub fn run_obr_with_env<I, S, E, K, V>(
    workspace: &ObrWorkspace,
    args: I,
    env_vars: E,
    label: &str,
) -> ObrRun
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    E: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    run_obr_full(workspace, args, env_vars, None, label)
}

pub fn run_obr_with_stdin<I, S>(
    workspace: &ObrWorkspace,
    args: I,
    input: &str,
    label: &str,
) -> ObrRun
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_obr_full(
        workspace,
        args,
        std::iter::empty::<(String, String)>(),
        Some(input),
        label,
    )
}

pub fn run_obr_smoke_at_root_with_env<I, S, E, K, V>(
    root: &Path,
    args: I,
    env_vars: E,
    label: &str,
) -> ObrRun
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    E: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    let log_dir = root.join("logs");
    run_obr_full_in_root(root, &log_dir, args, env_vars, None, label, true)
}

fn run_obr_full<I, S, E, K, V>(
    workspace: &ObrWorkspace,
    args: I,
    env_vars: E,
    stdin_input: Option<&str>,
    label: &str,
) -> ObrRun
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    E: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    run_obr_full_in_root(
        &workspace.root,
        &workspace.log_dir,
        args,
        env_vars,
        stdin_input,
        label,
        false,
    )
}

fn configure_test_command_environment<E, K, V>(cmd: &mut Command, root: &Path, env_vars: E)
where
    E: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    // Default e2e runs un-throttled so history-mechanics tests (backup
    // chronology, prune, restore) observe one `history/` snapshot per
    // mutation. Set before caller `env_vars` so a test can override this to
    // exercise the #313 snapshot throttle.
    cmd.env("OBR_HISTORY_MIN_INTERVAL_SECS", "0");
    // Keep each fixture's Git configuration hermetic by default, while still
    // allowing tests of effective global configuration to supply another
    // HOME explicitly.
    cmd.env("HOME", root);
    // `error`, not `obr=debug`. Debug tracing goes to stderr, which
    // (a) `obr doctor`'s own `rust_log` check flags as an agent-hostile
    // setting — so every "healthy workspace" doctor assertion failed purely
    // because the harness set it — and (b) drowns the assertions that match
    // on stderr contents. Tests that specifically want verbose tracing pass
    // RUST_LOG through the caller `env_vars`, which are applied after these
    // defaults and so still win.
    cmd.env("RUST_LOG", "error");
    // PATH is a *default*, not an override, so it must be set before the
    // caller `env_vars` rather than after. Applied afterwards it silently
    // discarded every caller-supplied PATH — which is exactly how the
    // Git-safety tests arm their probes: a directory holding a fake `git`
    // sentinel, or an empty directory that hides `git` entirely. Those
    // overrides have to survive for the assertions to mean anything; the
    // deduplicated PATH is only the fallback for callers that pass none.
    cmd.env("PATH", deduplicated_obr_path());
    cmd.envs(env_vars);
    cmd.env("NO_COLOR", "1");
    cmd.env("RUST_BACKTRACE", "1");
}

fn run_obr_full_in_root<I, S, E, K, V>(
    root: &Path,
    log_dir: &Path,
    args: I,
    env_vars: E,
    stdin_input: Option<&str>,
    label: &str,
    preserve_smoke_env: bool,
) -> ObrRun
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    E: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    fs::create_dir_all(log_dir).expect("log dir");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("obr"));
    cmd.current_dir(root);
    cmd.args(args);
    if preserve_smoke_env {
        clear_inherited_obr_env_except(&mut cmd, SMOKE_PRESERVED_ENV_KEYS);
    } else {
        clear_inherited_obr_env(&mut cmd);
    }
    configure_test_command_environment(&mut cmd, root, env_vars);

    if let Some(input) = stdin_input {
        cmd.write_stdin(input);
    }

    let start = Instant::now();
    let output = cmd.output().expect("run obr");
    let duration = start.elapsed();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let log_path = log_dir.join(format!("{label}.log"));
    let timestamp = SystemTime::now();
    let log_body = format!(
        "label: {label}\nstarted: {:?}\nduration: {:?}\nstatus: {}\nargs: {:?}\ncwd: {}\n\nstdout:\n{}\n\nstderr:\n{}\n",
        timestamp,
        duration,
        output.status,
        cmd.get_args().collect::<Vec<_>>(),
        root.display(),
        stdout,
        stderr
    );
    fs::write(&log_path, log_body).expect("write log");

    ObrRun {
        stdout,
        stderr,
        status: output.status,
        duration,
        log_path,
    }
}

/// Extract the issue ID from `obr create` stdout.
///
/// Handles both formats: `"Created pfx-xxx: title"` and `"✓ Created pfx-xxx: title"`.
pub fn parse_created_id(stdout: &str) -> String {
    let line = stdout.lines().next().unwrap_or("");
    let normalized = line.strip_prefix("✓ ").unwrap_or(line);
    normalized
        .strip_prefix("Created ")
        .and_then(|rest| rest.split(':').next())
        .unwrap_or("")
        .trim()
        .to_string()
}

pub fn extract_json_payload(stdout: &str) -> String {
    let lines: Vec<&str> = stdout.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') || trimmed.starts_with('{') {
            return lines[idx..].join("\n").trim().to_string();
        }
    }
    stdout.trim().to_string()
}

pub fn parse_json_value(stdout: &str) -> Value {
    let payload = extract_json_payload(stdout);
    serde_json::from_str(&payload).expect("valid JSON payload")
}

/// Extract an issue array from JSON stdout, handling both formats:
/// - Paginated: `{"issues": [...], "total": N, ...}` → returns the inner array
/// - Bare array: `[...]` → returns it directly
pub fn extract_issues_array(stdout: &str) -> Vec<Value> {
    let json = parse_json_value(stdout);
    if let Some(arr) = json.as_array() {
        return arr.clone();
    }
    if let Some(issues) = json.get("issues").and_then(Value::as_array) {
        return issues.clone();
    }
    panic!(
        "JSON output is neither a bare array nor an object with 'issues': {}",
        &stdout[..stdout.len().min(200)]
    );
}

pub fn parse_list_page(stdout: &str) -> Value {
    let json = parse_json_value(stdout);
    assert!(
        json.is_object(),
        "list JSON should be an object with pagination metadata"
    );
    assert!(
        json.get("issues").is_some(),
        "list JSON should contain an issues field"
    );
    json
}

pub fn parse_list_issues(stdout: &str) -> Vec<Value> {
    parse_list_page(stdout)
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .expect("list JSON should contain an issues array")
}

#[cfg(test)]
mod tests {
    use super::{
        ObrWorkspace, WORKSPACE_DIR_NAMES, configure_test_command_environment,
        is_inside_obr_workspace, isolated_temp_root, should_clear_inherited_obr_env,
        should_preserve_smoke_env,
    };
    use assert_cmd::Command;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn caller_home_overrides_the_hermetic_default() {
        let mut command = Command::new("obr");
        configure_test_command_environment(
            &mut command,
            Path::new("/fixture-default-home"),
            [("HOME", "/caller-selected-home")],
        );

        let home = command
            .get_envs()
            .find_map(|(key, value)| (key == OsStr::new("HOME")).then_some(value))
            .flatten();
        assert_eq!(home, Some(OsStr::new("/caller-selected-home")));
    }

    #[test]
    fn inherited_obr_and_toon_env_are_cleared() {
        for key in [
            "OBR_ACTOR",
            "OBR_CACHE_DIR",
            "OBR_JSONL",
            "BD_ACTOR",
            "BEADS_CACHE_DIR",
            "BEADS_JSONL",
            "BR_DISABLE_READ_ONLY_FAST_OPEN",
            "BR_OUTPUT_FORMAT",
            "TOON_DEFAULT_FORMAT",
            "TOON_STATS",
        ] {
            assert!(
                should_clear_inherited_obr_env(OsStr::new(key)),
                "{key} should be cleared for hermetic obr tests"
            );
        }
    }

    #[test]
    fn unrelated_env_are_preserved() {
        for key in ["HOME", "PATH", "RUST_LOG", "NO_COLOR"] {
            assert!(
                !should_clear_inherited_obr_env(OsStr::new(key)),
                "{key} should not be blanket-cleared"
            );
        }
    }

    #[test]
    fn smoke_profile_preserves_selected_routing_and_output_env() {
        for key in [
            "BEADS_DIR",
            "BEADS_CACHE_DIR",
            "BEADS_JSONL",
            "BR_OUTPUT_FORMAT",
            "TOON_DEFAULT_FORMAT",
            "TOON_STATS",
        ] {
            assert!(
                should_preserve_smoke_env(OsStr::new(key)),
                "{key} should be preserved for non-hermetic smoke coverage"
            );
        }
    }

    #[test]
    fn smoke_profile_still_clears_unrelated_legacy_beads_env() {
        for key in ["BD_ACTOR", "BD_DB", "BD_CONFIG", "BEADS_DEBUG"] {
            assert!(
                !should_preserve_smoke_env(OsStr::new(key)),
                "{key} should still be scrubbed in smoke mode"
            );
        }
    }

    /// Built from a synthetic tree rather than the obr checkout: rch
    /// excludes `.obr/` when syncing to remote workers, so "the repo tracks its
    /// own issues" does not hold everywhere the suite runs.
    #[test]
    fn detects_paths_enclosed_by_a_obr_workspace() {
        let root = TempDir::new_in(isolated_temp_root()).expect("temp dir");
        let nested = root.path().join("a/b/c");
        fs::create_dir_all(&nested).expect("nested dirs");
        assert!(
            !is_inside_obr_workspace(&nested),
            "a plain temp tree must not look like a workspace"
        );

        for marker in WORKSPACE_DIR_NAMES {
            let workspace_marker = root.path().join(marker);
            fs::create_dir_all(&workspace_marker).expect("workspace marker");
            assert!(
                is_inside_obr_workspace(&nested),
                "a descendant of a `{marker}` workspace must be detected"
            );
            fs::remove_dir(&workspace_marker).expect("drop workspace marker");
        }
    }

    /// Whatever root the harness picks must be usable and beads-free, otherwise
    /// every "uninitialized workspace" assertion in the suite is meaningless.
    #[test]
    fn isolated_temp_root_is_obr_free_and_usable() {
        let root = isolated_temp_root();
        assert!(
            root.is_dir(),
            "temp root {} is not a directory",
            root.display()
        );
        assert!(
            !is_inside_obr_workspace(&root),
            "temp root {} sits inside a obr workspace",
            root.display()
        );

        let workspace = ObrWorkspace::new();
        assert!(
            !is_inside_obr_workspace(&workspace.root),
            "workspace {} sits inside a obr workspace",
            workspace.root.display()
        );
    }
}
