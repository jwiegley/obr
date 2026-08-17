use crate::error::{BeadsError, Result};
use crate::output::{OutputContext, OutputMode};
use crate::storage::SqliteStorage;
use rich_rust::prelude::*;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Resolve the workspace directory for init.
///
/// Honors `OBR_DIR` if set. Otherwise `.obr` is created — except when
/// `base_dir` already holds a pre-rename workspace, in which case that
/// directory is returned so init reports the workspace as already initialized
/// instead of creating a second one beside it.
///
/// An empty `OBR_DIR=` counts as unset, matching
/// [`crate::config::discover_obr_dir`]: every other command filters the empty
/// value, and only this resolver turned it into a workspace path of `""`.
fn resolve_init_obr_dir(base_dir: &Path) -> PathBuf {
    resolve_init_obr_dir_with_override(
        base_dir,
        std::env::var(crate::config::WORKSPACE_DIR_ENV)
            .ok()
            .as_deref(),
    )
}

/// [`resolve_init_obr_dir`] with the environment override passed in, so the
/// rule is testable without mutating process-wide state.
fn resolve_init_obr_dir_with_override(base_dir: &Path, override_value: Option<&str>) -> PathBuf {
    if let Some(value) = override_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return PathBuf::from(value);
    }
    let preferred = base_dir.join(crate::config::WORKSPACE_DIR_NAME);
    if preferred.is_dir() {
        return preferred;
    }
    // `_obr` first, then the pre-rename `.beads`/`_beads` as legacy reads so
    // an un-migrated workspace is still found. `.obr` is handled above.
    for legacy in [
        crate::config::WORKSPACE_DIR_NAME_UNDERSCORE,
        ".beads",
        "_beads",
    ] {
        let candidate = base_dir.join(legacy);
        if candidate.is_dir() {
            return candidate;
        }
    }
    preferred
}

fn path_entry_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn create_temp_init_file(path: &Path) -> Result<(PathBuf, File)> {
    let pid = std::process::id();
    let base_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("tmp");

    for attempt in 0..64_u32 {
        let extension = if attempt == 0 {
            format!("{base_extension}.{pid}.tmp")
        } else {
            format!("{base_extension}.{pid}.{attempt}.tmp")
        };
        let temp_path = path.with_extension(extension);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }

    Err(BeadsError::Config(format!(
        "Failed to allocate temp init file for {}",
        path.display()
    )))
}

fn write_init_file_if_missing(path: &Path, contents: &[u8]) -> Result<bool> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(contents) {
                drop(file);
                let _ = fs::remove_file(path);
                return Err(error.into());
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn write_init_file_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    let existing_permissions = fs::symlink_metadata(path)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .map(|metadata| metadata.permissions());
    let (temp_path, mut temp_file) = create_temp_init_file(path)?;
    if let Some(permissions) = existing_permissions
        && let Err(error) = fs::set_permissions(&temp_path, permissions)
    {
        tracing::warn!(
            path = %path.display(),
            error = %error,
            "Failed to apply original init file permissions before atomic rewrite"
        );
    }
    if let Err(error) = temp_file
        .write_all(contents)
        .and_then(|()| temp_file.sync_all())
    {
        drop(temp_file);
        let _ = fs::remove_file(&temp_path);
        return Err(error.into());
    }
    drop(temp_file);
    crate::util::durable_rename(&temp_path, path).inspect_err(|_| {
        let _ = fs::remove_file(&temp_path);
    })?;
    Ok(())
}

/// The database this workspace uses, resolved through the config layer.
///
/// `init` asks the same resolver every other command asks. A second resolver
/// used to live in `util` for this call site alone, and its legacy `beads.db`
/// fallback was a bare existence probe where config's is gated on
/// `metadata.json` not naming a database — so `obr init` could open and record
/// `beads.db` in a workspace whose metadata pinned `obr.db`, and every later
/// command would then open a different, empty database.
fn resolved_db_path(obr_dir: &Path) -> Result<PathBuf> {
    Ok(crate::config::ConfigPaths::resolve(obr_dir, None)?.db_path)
}

/// Everything `obr init` actually did, in the terms the report must state it.
///
/// The reporting half of this command is handed only this value, so every
/// user-visible label is forced to derive from a path init really resolved.
/// That is the whole point of the split: the Rich panel used to carry literal
/// `.beads/`, `beads.db` and `issues.org` strings that survived three renames
/// and the D-SURFACE move because nothing tied them to the filesystem work.
struct InitFacts {
    /// The workspace directory as init used it: `.obr`, `_obr`, an adopted
    /// legacy `.beads`/`_beads`, or whatever `OBR_DIR` named.
    obr_dir: PathBuf,
    /// Project root the surface is reported relative to.
    root: PathBuf,
    db_path: PathBuf,
    metadata_path: PathBuf,
    config_path: PathBuf,
    gitignore_path: PathBuf,
    /// The tracked export surface — a SIBLING of `obr_dir`, never inside it.
    surface_path: PathBuf,
    dir_created: bool,
    db_created: bool,
    metadata_status: InitStepStatus,
    config_created: bool,
    gitignore_created: bool,
    surface_created: bool,
    prefix: String,
    /// Whether the prefix came from `--prefix` rather than the directory name.
    prefix_explicit: bool,
}

/// The final component of `path`, or its whole rendering when it has none.
fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

impl InitFacts {
    /// The workspace directory as the report names it, e.g. `.obr/`.
    fn workspace_dir_label(&self) -> String {
        format!("{}/", file_label(&self.obr_dir))
    }

    /// The database filename actually opened: `obr.db`, a name pinned in
    /// `metadata.json`, or the legacy `beads.db` of an adopted workspace.
    fn db_label(&self) -> String {
        file_label(&self.db_path)
    }

    /// The surface relative to the project root: `PLAN.org`, `doc/PLAN.org`,
    /// `docs/PLAN.org`. Falls back to the full path if it is not under the
    /// root (only reachable through an out-of-tree `OBR_DIR`).
    fn surface_label(&self) -> String {
        self.surface_path.strip_prefix(&self.root).map_or_else(
            |_| self.surface_path.display().to_string(),
            |relative| relative.display().to_string(),
        )
    }

    /// Whether the database sits directly inside the workspace directory.
    /// `OBR_CACHE_DIR` moves it out, and the layout must not claim otherwise.
    fn db_inside_workspace(&self) -> bool {
        self.db_path.parent() == Some(self.obr_dir.as_path())
    }
}

/// Execute the init command.
///
/// # Errors
///
/// Returns an error if the directory or database cannot be created.
pub fn execute(
    prefix: Option<String>,
    force: bool,
    root_dir: Option<&Path>,
    ctx: &OutputContext,
) -> Result<()> {
    let facts = init_workspace(prefix, force, root_dir)?;
    report_init(&facts, ctx);
    Ok(())
}

/// Do the filesystem work and report exactly which paths it touched.
///
/// # Errors
///
/// Returns an error if the directory or database cannot be created.
#[allow(clippy::too_many_lines)]
fn init_workspace(
    prefix: Option<String>,
    force: bool,
    root_dir: Option<&Path>,
) -> Result<InitFacts> {
    let base_dir = root_dir.unwrap_or_else(|| Path::new("."));
    let obr_dir = resolve_init_obr_dir(base_dir);

    let mut created_dir = false;
    if obr_dir.exists() {
        // Check if the database exists (in the cache dir if OBR_CACHE_DIR is set)
        let effective_db_path = resolved_db_path(&obr_dir)?;
        if effective_db_path.exists() && !force {
            return Err(BeadsError::AlreadyInitialized {
                path: effective_db_path,
            });
        }
    } else {
        fs::create_dir(&obr_dir)?;
        created_dir = true;
    }

    let effective_db_path = resolved_db_path(&obr_dir)?;
    let db_existed = effective_db_path.exists();

    // Ensure cache directory exists if using OBR_CACHE_DIR
    if let Some(parent) = effective_db_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Initialize DB (creates file and applies schema)
    let mut storage = SqliteStorage::open(&effective_db_path)?;

    // Set prefix in config table if provided, otherwise derive from directory name
    // Normalize to lowercase since ID validation requires lowercase prefixes
    let prefix_explicit = prefix.is_some();
    let actual_prefix = prefix.unwrap_or_else(|| {
        let mut dir_name = crate::util::id::DEFAULT_ISSUE_PREFIX.to_string();
        if let Ok(canon) = dunce::canonicalize(base_dir)
            && let Some(name) = canon.file_name().and_then(|n| n.to_str())
        {
            let cleaned: String = name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !cleaned.is_empty() {
                dir_name = cleaned;
            }
        }
        dir_name
    });
    let normalized = actual_prefix.to_ascii_lowercase();
    storage.set_config("issue_prefix", &normalized)?;

    // Write metadata.json
    let metadata_path = obr_dir.join("metadata.json");
    let metadata_existed = path_entry_exists(&metadata_path)?;
    // Record the database file that was actually opened: on a pre-rename
    // workspace `db_path` resolves to `beads.db`, and metadata must agree
    // rather than name a file that is not there.
    let database_name = effective_db_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(crate::config::DEFAULT_DB_FILENAME);
    let metadata = format!(
        "{{\n  \"database\": \"{database_name}\",\n  \"jsonl_export\": \"{}\"\n}}",
        crate::config::SURFACE_FILENAME
    );
    if force {
        write_init_file_atomically(&metadata_path, metadata.as_bytes())?;
    } else if !metadata_existed {
        write_init_file_if_missing(&metadata_path, metadata.as_bytes())?;
    }
    let metadata_status = if metadata_existed {
        if force {
            InitStepStatus::Updated
        } else {
            InitStepStatus::Existing
        }
    } else {
        InitStepStatus::Created
    };

    // Write config.yaml template
    let config_path = obr_dir.join("config.yaml");
    let config_existed = path_entry_exists(&config_path)?;
    let mut config_created = false;
    if !config_existed {
        let config = format!(
            "# obr project configuration
# issue_prefix: {normalized}
# default_priority: 2
# default_type: task
"
        );
        config_created = write_init_file_if_missing(&config_path, config.as_bytes())?;
    }

    // Write .gitignore
    let gitignore_path = obr_dir.join(".gitignore");
    let gitignore_existed = path_entry_exists(&gitignore_path)?;
    let mut gitignore_created = false;
    if !gitignore_existed {
        // D-SURFACE: `.obr/` is a per-machine cache. Keep upstream's
        // create-if-missing guard, but ignore this cache wholesale; PLAN.org
        // is seeded outside the workspace below as the tracked surface.
        let gitignore = format!(
            "# obr keeps only per-machine cache here; nothing under .obr/ is tracked.\n{}\n",
            crate::config::WORKSPACE_SELF_IGNORE_PATTERN
        );
        gitignore_created = write_init_file_if_missing(&gitignore_path, gitignore.as_bytes())?;
    }

    // Seed the tracked surface (D-SURFACE) so discovery and auto-import have a
    // well-defined empty export from the first command onward. An EXISTING
    // surface always wins, which is why this resolves rather than computes:
    // `obr init` in a fresh clone that already has `<root>/PLAN.org` and a
    // `doc/` directory would otherwise seed an empty `doc/PLAN.org`, which
    // outranks the tracked file and hides every issue in it. Only when no
    // surface exists at all does this fall through to the computed location,
    // in strict precedence: `<root>/doc/PLAN.org` if the project has a `doc/`,
    // else `<root>/docs/PLAN.org` if it has `docs/`, else `<root>/PLAN.org`.
    // obr never creates either directory itself. Nothing is seeded inside
    // `.obr/` any more — that directory is cache only.
    let root = crate::config::workspace_root_of(&obr_dir).unwrap_or_else(|| obr_dir.clone());
    let jsonl_path = crate::config::workspace_root_of(&obr_dir).map_or_else(
        || obr_dir.join(crate::config::SURFACE_FILENAME),
        |root| crate::config::resolve_surface_path(&root),
    );
    let jsonl_existed = path_entry_exists(&jsonl_path)?;
    let mut surface_created = false;
    if !jsonl_existed {
        surface_created = write_init_file_if_missing(&jsonl_path, b"")?;
        if surface_created {
            // Seeding the surface without certifying it left the workspace
            // unable to flush: the no-op branch of `execute_flush` requires a
            // stored content hash and the schema seeds that row empty. See
            // `certify_seeded_empty_surface` — it re-checks that the database
            // is empty, so an existing workspace whose surface was deleted and
            // re-seeded here is not stamped as certified.
            crate::sync::certify_seeded_empty_surface(&mut storage, &jsonl_path)?;
        }
    }

    Ok(InitFacts {
        obr_dir,
        root,
        db_path: effective_db_path,
        metadata_path,
        config_path,
        gitignore_path,
        surface_path: jsonl_path,
        dir_created: created_dir,
        db_created: !db_existed,
        metadata_status,
        config_created,
        gitignore_created,
        surface_created,
        prefix: normalized,
        prefix_explicit,
    })
}

/// Tell the user what [`init_workspace`] just did, in the requested mode.
fn report_init(facts: &InitFacts, ctx: &OutputContext) {
    match ctx.mode() {
        OutputMode::Quiet => {}
        OutputMode::Rich => render_init_rich(facts, ctx),
        OutputMode::Plain | OutputMode::Json | OutputMode::Toon => {
            println!("Prefix set to: {}", facts.prefix);
            println!("Initialized obr workspace in {}", facts.obr_dir.display());
            // The surface is the one artifact of this command that lands in
            // git, so plain output has to name it — and name the resolved one,
            // which is `doc/PLAN.org` in a project that has a `doc/`.
            println!("Plan surface: {}", facts.surface_label());
        }
    }
}

#[derive(Clone, Copy)]
enum InitStepStatus {
    Created,
    Updated,
    Existing,
}

struct InitStep {
    label: String,
    status: InitStepStatus,
}

/// The per-artifact checklist, every label read off a path init resolved.
fn build_init_steps(facts: &InitFacts) -> Vec<InitStep> {
    let created = |created: bool| {
        if created {
            InitStepStatus::Created
        } else {
            InitStepStatus::Existing
        }
    };

    let mut steps = vec![
        InitStep {
            label: format!("{} directory", facts.workspace_dir_label()),
            status: created(facts.dir_created),
        },
        InitStep {
            label: format!("SQLite database ({})", facts.db_label()),
            status: created(facts.db_created),
        },
        InitStep {
            label: file_label(&facts.metadata_path),
            status: facts.metadata_status,
        },
        InitStep {
            label: file_label(&facts.config_path),
            status: created(facts.config_created),
        },
        InitStep {
            label: file_label(&facts.gitignore_path),
            status: created(facts.gitignore_created),
        },
        InitStep {
            label: format!("{} (empty export seed)", facts.surface_label()),
            status: created(facts.surface_created),
        },
    ];

    steps.push(InitStep {
        label: format!("Issue prefix set to '{}'", facts.prefix),
        status: InitStepStatus::Updated,
    });

    steps
}

/// The layout tree, drawn from the real directory name and the real file set.
///
/// D-SURFACE: the export surface is a SIBLING of the workspace directory —
/// tracked in git, at the project root or under `doc/`/`docs/` — so it is
/// drawn at the top level, never as a child of the cache directory. Likewise
/// the database only appears inside the directory when it really is there;
/// `OBR_CACHE_DIR` moves it elsewhere and it is then listed on its own.
fn build_init_layout_lines(facts: &InitFacts) -> Vec<String> {
    let db_inside = facts.db_inside_workspace();
    let mut contents = Vec::new();
    if db_inside {
        contents.push(facts.db_label());
    }
    contents.push(file_label(&facts.metadata_path));
    contents.push(file_label(&facts.config_path));
    contents.push(file_label(&facts.gitignore_path));

    let mut lines = vec![format!("  {}", facts.workspace_dir_label())];
    let last = contents.len().saturating_sub(1);
    for (index, name) in contents.iter().enumerate() {
        let connector = if index == last { "`--" } else { "|--" };
        lines.push(format!("    {connector} {name}"));
    }

    lines.push(format!("  {}", facts.surface_label()));
    if !db_inside {
        lines.push(format!("  {}", facts.db_path.display()));
    }

    lines
}

/// The full Rich panel body. Pure: it reads only resolved facts and a theme.
fn build_init_panel_text(facts: &InitFacts, theme: &crate::output::Theme) -> Text {
    let mut content = Text::new("");

    content.append_styled("Workspace initialized\n", theme.emphasis.clone());
    content.append("\n");

    content.append_styled("Location: ", theme.dimmed.clone());
    content.append_styled(&facts.obr_dir.display().to_string(), theme.accent.clone());
    content.append("\n\n");

    content.append_styled("Steps:\n", theme.emphasis.clone());
    for step in &build_init_steps(facts) {
        append_step(&mut content, step, theme);
    }

    content.append("\n");
    content.append_styled("Layout:\n", theme.emphasis.clone());
    for line in build_init_layout_lines(facts) {
        content.append(&line);
        content.append("\n");
    }

    content.append("\n");
    content.append_styled("Next steps:\n", theme.emphasis.clone());
    content.append("  obr create \"My first issue\"\n");
    content.append("  obr list\n");

    // A prefix is always set; the tip is only true advice when obr picked it
    // from the directory name rather than the user naming one.
    if !facts.prefix_explicit {
        content.append("\n");
        content.append_styled(
            "Tip: Set a custom prefix with `obr init --prefix <name>`\n",
            theme.dimmed.clone(),
        );
    }

    content
}

/// Frame the panel body. Shared with the tests so what they capture is what
/// the terminal gets, not a look-alike rebuilt beside it.
fn build_init_panel<'a>(
    content: &'a Text,
    theme: &crate::output::Theme,
    width: usize,
) -> Panel<'a> {
    Panel::from_rich_text(content, width)
        .title(Text::new("Obr Initialized"))
        .box_style(theme.box_style)
        .border_style(theme.panel_border.clone())
}

fn render_init_rich(facts: &InitFacts, ctx: &OutputContext) {
    let theme = ctx.theme();
    let content = build_init_panel_text(facts, theme);
    ctx.render(&build_init_panel(&content, theme, ctx.width()));
}

fn append_step(content: &mut Text, step: &InitStep, theme: &crate::output::Theme) {
    let (icon, style) = match step.status {
        InitStepStatus::Created => ("[+]", theme.success.clone()),
        InitStepStatus::Updated => ("[*]", theme.warning.clone()),
        InitStepStatus::Existing => ("[=]", theme.dimmed.clone()),
    };
    content.append_styled(&format!("{icon} "), style);
    content.append_styled(&step.label, theme.issue_title.clone());
    content.append("\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tracing::info;

    fn init_logging() {
        crate::logging::init_test_logging();
    }

    /// Wide enough that no fact wraps inside the panel frame, so a substring
    /// assertion means the panel really said it.
    const PANEL_WIDTH: usize = 200;

    /// Names that were true before the rename and the D-SURFACE move, and are
    /// lies for any workspace obr creates today.
    const STALE_LITERALS: &[&str] = &[".beads", "beads.db", "issues.org"];

    /// Render the init panel through the REAL Rich path and return its text.
    ///
    /// `OutputMode::Rich` is normally only reachable on an interactive TTY,
    /// which is exactly why every string in this panel went untested through
    /// four rename passes. `OutputContext::with_mode` forces it in-process,
    /// and the panel is built by the same two functions `render_init_rich`
    /// calls — not a look-alike rebuilt for the test.
    fn capture_rich_panel(facts: &InitFacts) -> String {
        let ctx = OutputContext::with_mode(OutputMode::Rich);
        assert!(ctx.is_rich(), "the panel must be exercised in Rich mode");
        let theme = ctx.theme();
        let content = build_init_panel_text(facts, theme);
        let panel = build_init_panel(&content, theme, PANEL_WIDTH);

        let console = Console::builder()
            .no_color()
            .force_terminal(true)
            .width(PANEL_WIDTH)
            .build();
        console.begin_capture();
        console.print_renderable(&panel);
        console
            .end_capture()
            .iter()
            .map(|segment| segment.text.as_ref())
            .collect::<String>()
            .replace('\\', "/")
    }

    fn assert_names_no_stale_artifact(rendered: &str, case: &str) {
        for stale in STALE_LITERALS {
            assert!(
                !rendered.contains(stale),
                "{case}: the panel still prints the pre-rename literal {stale:?}:\n{rendered}"
            );
        }
    }

    fn layout_lines(facts: &InitFacts) -> Vec<String> {
        build_init_layout_lines(facts)
            .into_iter()
            .map(|line| line.replace('\\', "/"))
            .collect()
    }

    /// (a) The default workspace: `.obr/` holding `obr.db`, with the tracked
    /// surface a SIBLING at the project root.
    #[test]
    fn rich_panel_names_the_real_default_workspace() {
        init_logging();
        let temp = TempDir::new().unwrap();
        let facts = init_workspace(Some("alpha".into()), false, Some(temp.path())).unwrap();

        assert!(temp.path().join(".obr/obr.db").is_file());
        assert!(temp.path().join("PLAN.org").is_file());

        let rendered = capture_rich_panel(&facts);
        assert_names_no_stale_artifact(&rendered, "default workspace");

        for expected in [
            ".obr/ directory",
            "SQLite database (obr.db)",
            "PLAN.org (empty export seed)",
            "Issue prefix set to 'alpha'",
        ] {
            assert!(
                rendered.contains(expected),
                "default workspace: panel is missing {expected:?}:\n{rendered}"
            );
        }

        assert_eq!(
            layout_lines(&facts),
            vec![
                "  .obr/",
                "    |-- obr.db",
                "    |-- metadata.json",
                "    |-- config.yaml",
                "    `-- .gitignore",
                // D-SURFACE: outside the cache directory, at the same level.
                "  PLAN.org",
            ],
        );
        for line in layout_lines(&facts) {
            assert!(
                rendered.contains(&line),
                "default workspace: layout line {line:?} missing from panel:\n{rendered}"
            );
        }

        // The explicit prefix makes the "set a custom prefix" tip a lie.
        assert!(!rendered.contains("Tip: Set a custom prefix"));
    }

    /// (b) A project that already has `doc/`: the surface resolves there, and
    /// the panel must say so rather than claim a root `PLAN.org`.
    #[test]
    fn rich_panel_names_the_doc_surface_when_the_project_has_one() {
        init_logging();
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("doc")).unwrap();
        let facts = init_workspace(None, false, Some(temp.path())).unwrap();

        assert!(temp.path().join("doc/PLAN.org").is_file());
        assert!(!temp.path().join("PLAN.org").exists());

        let rendered = capture_rich_panel(&facts);
        assert_names_no_stale_artifact(&rendered, "doc surface");

        assert!(
            rendered.contains("doc/PLAN.org (empty export seed)"),
            "doc surface: panel must name the resolved surface:\n{rendered}"
        );
        assert_eq!(
            layout_lines(&facts).last().map(String::as_str),
            Some("  doc/PLAN.org"),
            "doc surface: the tree must show the surface where it landed"
        );
        // A derived prefix is the one case where the tip is honest advice.
        assert!(
            rendered.contains("Tip: Set a custom prefix"),
            "doc surface: a derived prefix should still offer the tip:\n{rendered}"
        );
    }

    /// (c) A pre-rename workspace adopted in place. Here `.beads/` and
    /// `beads.db` are the TRUTH, and the panel must print them — the same
    /// derivation that forbids them above requires them here.
    #[test]
    fn rich_panel_names_an_adopted_legacy_workspace() {
        init_logging();
        let temp = TempDir::new().unwrap();

        // Build a REAL workspace, then rename it back to the pre-P6-02/03
        // names and un-record the database, so metadata is defaulted exactly
        // as it is in a workspace created before the rename.
        init_workspace(Some("legacy".into()), false, Some(temp.path())).unwrap();
        let legacy_dir = temp.path().join(".beads");
        fs::rename(temp.path().join(".obr"), &legacy_dir).unwrap();
        fs::rename(legacy_dir.join("obr.db"), legacy_dir.join("beads.db")).unwrap();
        fs::write(
            legacy_dir.join("metadata.json"),
            br#"{"jsonl_export":"PLAN.org"}"#,
        )
        .unwrap();
        fs::remove_file(temp.path().join("PLAN.org")).unwrap();

        let facts = init_workspace(Some("legacy".into()), true, Some(temp.path())).unwrap();

        assert!(
            legacy_dir.join("beads.db").is_file(),
            "the legacy database must be adopted in place, never renamed"
        );
        assert!(!temp.path().join(".obr").exists());

        let rendered = capture_rich_panel(&facts);
        for expected in [
            ".beads/ directory",
            "SQLite database (beads.db)",
            "PLAN.org (empty export seed)",
        ] {
            assert!(
                rendered.contains(expected),
                "legacy workspace: panel must report what it really used, missing \
                 {expected:?}:\n{rendered}"
            );
        }
        assert_eq!(
            layout_lines(&facts),
            vec![
                "  .beads/",
                "    |-- beads.db",
                "    |-- metadata.json",
                "    |-- config.yaml",
                "    `-- .gitignore",
                "  PLAN.org",
            ],
        );
        assert!(
            !rendered.contains("issues.org"),
            "even a legacy workspace seeds the surface outside the dot dir:\n{rendered}"
        );
    }

    /// The plain (non-TTY) branch prints its own strings. They are derived
    /// from the same facts, so this pins the three values it emits.
    #[test]
    fn plain_report_values_are_derived_from_the_workspace() {
        init_logging();
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("docs")).unwrap();
        let facts = init_workspace(Some("plainy".into()), false, Some(temp.path())).unwrap();

        assert_eq!(facts.prefix, "plainy");
        assert_eq!(
            facts.obr_dir,
            temp.path().join(crate::config::WORKSPACE_DIR_NAME)
        );
        assert_eq!(facts.surface_label().replace('\\', "/"), "docs/PLAN.org");
        assert!(temp.path().join("docs/PLAN.org").is_file());
    }

    /// A step may only claim "created" for a file this run actually created.
    #[test]
    fn a_second_init_claims_nothing_new() {
        init_logging();
        let temp = TempDir::new().unwrap();
        init_workspace(Some("twice".into()), false, Some(temp.path())).unwrap();
        let facts = init_workspace(Some("twice".into()), true, Some(temp.path())).unwrap();

        assert!(!facts.dir_created);
        assert!(!facts.db_created);
        assert!(!facts.config_created);
        assert!(!facts.gitignore_created);
        assert!(!facts.surface_created);
        assert!(matches!(facts.metadata_status, InitStepStatus::Updated));

        let rendered = capture_rich_panel(&facts);
        assert_names_no_stale_artifact(&rendered, "re-init");
        // `[+]` is the created marker; only the forced metadata rewrite `[*]`
        // and the prefix line may be non-`[=]` on a re-init.
        assert!(
            !rendered.contains("[+]"),
            "re-init must not claim to have created anything:\n{rendered}"
        );
    }

    #[test]
    fn test_init_creates_obr_directory() {
        init_logging();
        info!("test_init_creates_obr_directory: starting");
        let temp_dir = TempDir::new().unwrap();
        let ctx = OutputContext::from_flags(false, false, true);
        let result = execute(None, false, Some(temp_dir.path()), &ctx);

        assert!(result.is_ok());
        // P6-02/03 renamed the workspace directory and database; P6-06 moved
        // the export out of it entirely, so the seeded surface is at the root.
        assert!(temp_dir.path().join(".obr").exists());
        assert!(temp_dir.path().join(".obr/obr.db").exists());
        assert!(temp_dir.path().join(".obr/metadata.json").exists());
        assert!(temp_dir.path().join(".obr/config.yaml").exists());
        assert!(temp_dir.path().join(".obr/.gitignore").exists());
        assert!(temp_dir.path().join("PLAN.org").exists());
        assert!(!temp_dir.path().join(".obr/issues.org").exists());
        info!("test_init_creates_obr_directory: assertions passed");
    }

    #[cfg(unix)]
    #[test]
    fn test_init_does_not_follow_dangling_config_symlink() {
        use std::os::unix::fs::symlink;

        init_logging();
        let temp_dir = TempDir::new().unwrap();
        let obr_dir = temp_dir.path().join(".obr");
        fs::create_dir(&obr_dir).unwrap();
        let outside_target = temp_dir.path().join("outside-config.yaml");
        let config_path = obr_dir.join("config.yaml");
        symlink(&outside_target, &config_path).unwrap();

        let ctx = OutputContext::from_flags(false, false, true);
        let result = execute(None, false, Some(temp_dir.path()), &ctx);

        assert!(result.is_ok());
        assert!(
            !outside_target.exists(),
            "init must not create a dangling symlink target outside .beads"
        );
        assert!(
            fs::symlink_metadata(&config_path)
                .unwrap()
                .file_type()
                .is_symlink(),
            "create-only init files should leave existing symlink entries alone"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_init_force_replaces_metadata_symlink_not_target() {
        use std::os::unix::fs::symlink;

        init_logging();
        let temp_dir = TempDir::new().unwrap();
        let obr_dir = temp_dir.path().join(".obr");
        fs::create_dir(&obr_dir).unwrap();
        let outside_target = temp_dir.path().join("outside-metadata.json");
        let metadata_path = obr_dir.join("metadata.json");
        symlink(&outside_target, &metadata_path).unwrap();

        let ctx = OutputContext::from_flags(false, false, true);
        let result = execute(None, true, Some(temp_dir.path()), &ctx);

        assert!(result.is_ok());
        assert!(
            !outside_target.exists(),
            "force init must replace the metadata symlink entry, not write its target"
        );
        assert!(
            fs::metadata(&metadata_path).unwrap().is_file(),
            "metadata should be installed as a regular file"
        );
        assert!(
            !fs::symlink_metadata(&metadata_path)
                .unwrap()
                .file_type()
                .is_symlink(),
            "metadata symlink should be replaced by the forced rewrite"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_init_force_preserves_metadata_permissions() {
        use std::os::unix::fs::PermissionsExt;

        init_logging();
        let temp_dir = TempDir::new().unwrap();
        let ctx = OutputContext::from_flags(false, false, true);
        execute(None, false, Some(temp_dir.path()), &ctx).unwrap();

        let metadata_path = temp_dir.path().join(".obr/metadata.json");
        fs::set_permissions(&metadata_path, fs::Permissions::from_mode(0o600)).unwrap();

        execute(None, true, Some(temp_dir.path()), &ctx).unwrap();

        let mode = fs::metadata(&metadata_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "force init should preserve metadata mode");
    }

    #[test]
    fn test_init_with_prefix() {
        init_logging();
        info!("test_init_with_prefix: starting");
        let temp_dir = TempDir::new().unwrap();
        let ctx = OutputContext::from_flags(false, false, true);
        let result = execute(Some("test".to_string()), false, Some(temp_dir.path()), &ctx);

        assert!(result.is_ok());

        // Verify prefix was stored
        let db_path = temp_dir.path().join(".obr/obr.db");
        let storage = SqliteStorage::open(&db_path).unwrap();
        let prefix = storage.get_config("issue_prefix").unwrap();
        assert_eq!(prefix, Some("test".to_string()));
        info!("test_init_with_prefix: assertions passed");
    }

    /// The digest of zero bytes, which is what both export formats compute for
    /// an empty surface (Org hashes raw bytes; JSONL hashes canonicalized
    /// lines, of which an empty file has none).
    const EMPTY_SURFACE_DIGEST: &str =
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn init_certifies_the_empty_surface_it_seeds() {
        init_logging();
        let temp_dir = TempDir::new().unwrap();
        let ctx = OutputContext::from_flags(false, false, true);
        execute(None, false, Some(temp_dir.path()), &ctx).unwrap();

        // Named for what it asserts. The *behavior* this protects — that
        // `obr init && obr sync --flush-only` succeeds — needs a real flush,
        // which is more than a unit test here can reach; the doctor fixtures
        // `doctor_runs_dir_growth` and `obr_history_growth` run exactly that
        // pair and were the failures that exposed this.
        //
        // `init` seeds an empty surface, so the flush takes the no-op branch
        // and demands the last certified content hash. The schema seeds that
        // row empty, so before this was certified the first flush in every new
        // workspace failed closed with "Cannot certify a no-op flush".
        let db_path = temp_dir.path().join(".obr/obr.db");
        let storage = SqliteStorage::open(&db_path).unwrap();
        assert_eq!(
            storage
                .get_metadata(crate::sync::METADATA_JSONL_CONTENT_HASH)
                .unwrap()
                .as_deref(),
            Some(EMPTY_SURFACE_DIGEST),
            "init must certify the empty surface it seeded"
        );
    }

    #[test]
    fn a_forced_reinit_never_certifies_a_surface_that_lost_issues() {
        init_logging();
        let temp_dir = TempDir::new().unwrap();
        let ctx = OutputContext::from_flags(false, false, true);
        execute(None, false, Some(temp_dir.path()), &ctx).unwrap();

        // Stand in for a workspace with real content: one issue in the
        // database, and the tracked surface gone from disk.
        let db_path = temp_dir.path().join(".obr/obr.db");
        {
            let mut storage = SqliteStorage::open(&db_path).unwrap();
            let issue = crate::model::Issue {
                id: "test-1".to_string(),
                title: "a real issue".to_string(),
                ..crate::model::Issue::default()
            };
            storage.create_issue(&issue, "test").unwrap();
            storage
                .set_metadata(
                    crate::sync::METADATA_JSONL_CONTENT_HASH,
                    "certified-earlier",
                )
                .unwrap();
        }
        fs::remove_file(temp_dir.path().join("PLAN.org")).unwrap();

        // `--force` re-seeds an empty surface over a database that still holds
        // the issue. Certifying that would publish an empty merge anchor and
        // hand the next flush a licence to erase the issue silently.
        execute(None, true, Some(temp_dir.path()), &ctx).unwrap();

        let storage = SqliteStorage::open(&db_path).unwrap();
        let stored = storage
            .get_metadata(crate::sync::METADATA_JSONL_CONTENT_HASH)
            .unwrap();
        assert_ne!(
            stored.as_deref(),
            Some(EMPTY_SURFACE_DIGEST),
            "a re-seeded empty surface must not be certified while the database has issues"
        );
        assert_eq!(stored.as_deref(), Some("certified-earlier"));
    }

    #[test]
    fn test_init_fails_if_already_initialized() {
        init_logging();
        info!("test_init_fails_if_already_initialized: starting");
        let temp_dir = TempDir::new().unwrap();
        let ctx = OutputContext::from_flags(false, false, true);

        // First init should succeed
        let result1 = execute(None, false, Some(temp_dir.path()), &ctx);
        assert!(result1.is_ok());

        // Second init without force should fail
        let result2 = execute(None, false, Some(temp_dir.path()), &ctx);

        assert!(result2.is_err());
        assert!(matches!(
            result2.unwrap_err(),
            BeadsError::AlreadyInitialized { .. }
        ));
        info!("test_init_fails_if_already_initialized: assertions passed");
    }

    #[test]
    fn test_init_force_overwrites_existing() {
        init_logging();
        info!("test_init_force_overwrites_existing: starting");
        let temp_dir = TempDir::new().unwrap();
        let ctx = OutputContext::from_flags(false, false, true);

        // First init
        execute(
            Some("first".to_string()),
            false,
            Some(temp_dir.path()),
            &ctx,
        )
        .unwrap();

        // Second init with force
        let result = execute(
            Some("second".to_string()),
            true,
            Some(temp_dir.path()),
            &ctx,
        );

        assert!(result.is_ok());

        // Verify new prefix
        let db_path = temp_dir.path().join(".obr/obr.db");
        let storage = SqliteStorage::open(&db_path).unwrap();
        let prefix = storage.get_config("issue_prefix").unwrap();
        assert_eq!(prefix, Some("second".to_string()));
        info!("test_init_force_overwrites_existing: assertions passed");
    }

    #[test]
    fn test_metadata_json_content() {
        init_logging();
        info!("test_metadata_json_content: starting");
        let temp_dir = TempDir::new().unwrap();
        let ctx = OutputContext::from_flags(false, false, true);
        execute(None, false, Some(temp_dir.path()), &ctx).unwrap();

        let metadata_path = temp_dir.path().join(".obr/metadata.json");
        let content = fs::read_to_string(metadata_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["database"], "obr.db");
        assert_eq!(parsed["jsonl_export"], "PLAN.org");
        info!("test_metadata_json_content: assertions passed");
    }

    #[test]
    fn test_gitignore_excludes_db_files() {
        init_logging();
        info!("test_gitignore_excludes_db_files: starting");
        let temp_dir = TempDir::new().unwrap();
        let ctx = OutputContext::from_flags(false, false, true);
        execute(None, false, Some(temp_dir.path()), &ctx).unwrap();

        let gitignore_path = temp_dir.path().join(".obr/.gitignore");
        let content = fs::read_to_string(gitignore_path).unwrap();

        // The whole directory is per-machine cache, so one wholesale rule
        // replaces the per-artifact globs this test used to enumerate.
        assert!(
            content.lines().any(|line| line.trim() == "*"),
            "init must emit the self-ignore rule expected by doctor: {content}"
        );
        info!("test_gitignore_excludes_db_files: assertions passed");
    }

    /// An empty `OBR_DIR=` means "unset", not "the workspace is at `\"\"`".
    ///
    /// Every other command filters the empty value
    /// (`obr_dir_override_from_env` in config); only this resolver took it
    /// literally, so `OBR_DIR= obr init` resolved the workspace to the empty
    /// path instead of falling through to `./.obr`.
    #[test]
    fn empty_workspace_dir_override_is_treated_as_unset() {
        let temp = TempDir::new().unwrap();
        let default = temp.path().join(crate::config::WORKSPACE_DIR_NAME);

        for empty in [Some(""), Some("   "), None] {
            assert_eq!(
                resolve_init_obr_dir_with_override(temp.path(), empty),
                default,
                "{empty:?} must fall through to the default workspace name"
            );
        }

        assert_eq!(
            resolve_init_obr_dir_with_override(temp.path(), Some(" /tmp/elsewhere ")),
            PathBuf::from("/tmp/elsewhere"),
            "a real override is honored, trimmed"
        );
    }

    /// `init` resolves the database through the config layer, so its answer is
    /// the config layer's answer by construction. The two cases that used to
    /// diverge, when `init` had its own resolver:
    ///
    /// - metadata does not name a database → the pre-rename `beads.db` is
    ///   opened in place (both resolvers agreed here);
    /// - metadata names `obr.db` → the legacy file is NOT adopted, even though
    ///   it is the only database present. `init`'s old resolver was a bare
    ///   existence probe and picked `beads.db`, so it recorded a database that
    ///   every other command would then refuse to resolve.
    #[test]
    fn init_resolves_the_database_through_the_config_layer() {
        let temp = TempDir::new().unwrap();
        let obr_dir = temp.path().join(crate::config::WORKSPACE_DIR_NAME);
        fs::create_dir_all(&obr_dir).unwrap();
        fs::write(obr_dir.join("beads.db"), b"stale legacy database").unwrap();

        // No recorded database name: the legacy file is the workspace's.
        fs::write(
            obr_dir.join("metadata.json"),
            br#"{"jsonl_export":"PLAN.org"}"#,
        )
        .unwrap();
        let resolved = resolved_db_path(&obr_dir).expect("resolve legacy");
        assert_eq!(
            resolved.file_name().and_then(|n| n.to_str()),
            Some("beads.db"),
            "a pre-rename workspace must keep opening its own database"
        );
        assert_eq!(
            resolved,
            crate::config::ConfigPaths::resolve(&obr_dir, None)
                .expect("config resolve")
                .db_path,
            "init and the config layer must agree"
        );

        // A recorded database name is authoritative: no legacy adoption.
        fs::write(
            obr_dir.join("metadata.json"),
            br#"{"database":"obr.db","jsonl_export":"PLAN.org"}"#,
        )
        .unwrap();
        let resolved = resolved_db_path(&obr_dir).expect("resolve pinned");
        assert_eq!(
            resolved.file_name().and_then(|n| n.to_str()),
            Some("obr.db"),
            "an explicitly recorded database name must not fall back to beads.db"
        );
        assert_eq!(
            resolved,
            crate::config::ConfigPaths::resolve(&obr_dir, None)
                .expect("config resolve")
                .db_path,
            "init and the config layer must agree"
        );
    }
}
