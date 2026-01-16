//! Configuration management for `beads_rust`.
//!
//! Configuration sources and precedence (highest wins):
//! 1. CLI overrides
//! 2. Environment variables
//! 3. Project config (.beads/config.yaml)
//! 4. User config (~/.config/bd/config.yaml)
//! 5. Legacy user config (~/.beads/config.yaml)
//! 6. DB config table
//! 7. Defaults

use crate::error::{BeadsError, Result};
use crate::storage::SqliteStorage;
use crate::util::id::IdConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Default database filename used when metadata is missing.
const DEFAULT_DB_FILENAME: &str = "beads.db";
/// Default JSONL filename used when metadata is missing.
const DEFAULT_JSONL_FILENAME: &str = "issues.jsonl";

/// Startup metadata describing DB + JSONL paths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Metadata {
    pub database: String,
    pub jsonl_export: String,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub deletions_retention_days: Option<u64>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            database: DEFAULT_DB_FILENAME.to_string(),
            jsonl_export: DEFAULT_JSONL_FILENAME.to_string(),
            backend: None,
            deletions_retention_days: None,
        }
    }
}

impl Metadata {
    /// Load metadata.json from the beads directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load(beads_dir: &Path) -> Result<Self> {
        let path = beads_dir.join("metadata.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(&path)?;
        let mut metadata: Self = serde_json::from_str(&contents)?;

        if metadata.database.trim().is_empty() {
            metadata.database = DEFAULT_DB_FILENAME.to_string();
        }
        if metadata.jsonl_export.trim().is_empty() {
            metadata.jsonl_export = DEFAULT_JSONL_FILENAME.to_string();
        }

        Ok(metadata)
    }
}

/// Resolved paths for this workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub beads_dir: PathBuf,
    pub db_path: PathBuf,
    pub jsonl_path: PathBuf,
    pub metadata: Metadata,
}

impl ConfigPaths {
    /// Resolve database + JSONL paths using metadata and environment overrides.
    ///
    /// # Errors
    ///
    /// Returns an error if metadata cannot be read.
    pub fn resolve(beads_dir: &Path, db_override: Option<&PathBuf>) -> Result<Self> {
        let metadata = Metadata::load(beads_dir)?;
        let db_path = resolve_db_path(beads_dir, &metadata, db_override);
        let jsonl_path = resolve_jsonl_path(beads_dir, &metadata, db_override);

        Ok(Self {
            beads_dir: beads_dir.to_path_buf(),
            db_path,
            jsonl_path,
            metadata,
        })
    }
}

/// Discover the active `.beads` directory.
///
/// Honors `BEADS_DIR` when set, otherwise walks up from `start` (or CWD).
///
/// # Errors
///
/// Returns an error if no beads directory is found or the CWD cannot be read.
pub fn discover_beads_dir(start: Option<&Path>) -> Result<PathBuf> {
    discover_beads_dir_with_env(start, None)
}

fn discover_beads_dir_with_env(
    start: Option<&Path>,
    env_override: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(path) = env_override {
        if path.is_dir() {
            return Ok(path.to_path_buf());
        }
    } else if let Ok(value) = env::var("BEADS_DIR") {
        if !value.trim().is_empty() {
            let path = PathBuf::from(value);
            if path.is_dir() {
                return Ok(path);
            }
        }
    }

    let mut current = match start {
        Some(path) => path.to_path_buf(),
        None => env::current_dir()?,
    };

    loop {
        let candidate = current.join(".beads");
        if candidate.is_dir() {
            return Ok(candidate);
        }

        if !current.pop() {
            break;
        }
    }

    Err(BeadsError::NotInitialized)
}

/// Open storage using resolved config paths, returning the storage and paths used.
///
/// # Errors
///
/// Returns an error if metadata cannot be read or the database cannot be opened.
pub fn open_storage(
    beads_dir: &Path,
    db_override: Option<&PathBuf>,
) -> Result<(SqliteStorage, ConfigPaths)> {
    let paths = ConfigPaths::resolve(beads_dir, db_override)?;
    let storage = SqliteStorage::open(&paths.db_path)?;
    Ok((storage, paths))
}

fn resolve_db_path(
    beads_dir: &Path,
    metadata: &Metadata,
    db_override: Option<&PathBuf>,
) -> PathBuf {
    if let Some(override_path) = db_override {
        return override_path.clone();
    }

    let candidate = PathBuf::from(&metadata.database);
    if candidate.is_absolute() {
        candidate
    } else {
        beads_dir.join(candidate)
    }
}

fn resolve_jsonl_path(
    beads_dir: &Path,
    metadata: &Metadata,
    db_override: Option<&PathBuf>,
) -> PathBuf {
    if let Ok(env_path) = env::var("BEADS_JSONL") {
        if !env_path.trim().is_empty() {
            return PathBuf::from(env_path);
        }
    }

    if db_override.is_some() {
        return db_override
            .and_then(|path| {
                path.parent()
                    .map(|parent| parent.join(DEFAULT_JSONL_FILENAME))
            })
            .unwrap_or_else(|| beads_dir.join(DEFAULT_JSONL_FILENAME));
    }

    let candidate = PathBuf::from(&metadata.jsonl_export);
    if candidate.is_absolute() {
        candidate
    } else {
        beads_dir.join(candidate)
    }
}

/// A configuration layer split into startup-only and runtime (DB) keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigLayer {
    pub startup: HashMap<String, String>,
    pub runtime: HashMap<String, String>,
}

impl ConfigLayer {
    /// Merge another layer on top of this one (higher precedence wins).
    pub fn merge_from(&mut self, other: &Self) {
        for (key, value) in &other.startup {
            self.startup.insert(key.clone(), value.clone());
        }
        for (key, value) in &other.runtime {
            self.runtime.insert(key.clone(), value.clone());
        }
    }

    /// Merge multiple layers in precedence order (lowest to highest).
    #[must_use]
    pub fn merge_layers(layers: &[Self]) -> Self {
        let mut merged = Self::default();
        for layer in layers {
            merged.merge_from(layer);
        }
        merged
    }

    /// Build a layer from a YAML file path. Missing files return empty config.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn from_yaml(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)?;
        let value: serde_yaml::Value = serde_yaml::from_str(&contents)?;
        Ok(layer_from_yaml_value(&value))
    }

    /// Build a layer from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        let mut layer = Self::default();

        for (key, value) in env::vars() {
            if let Some(stripped) = key.strip_prefix("BD_") {
                let normalized = stripped.to_lowercase();
                for variant in env_key_variants(&normalized) {
                    insert_key_value(&mut layer, &variant, value.clone());
                }
            }
        }

        if let Ok(value) = env::var("BEADS_FLUSH_DEBOUNCE") {
            insert_key_value(&mut layer, "flush-debounce", value);
        }
        if let Ok(value) = env::var("BEADS_IDENTITY") {
            insert_key_value(&mut layer, "identity", value);
        }
        if let Ok(value) = env::var("BEADS_REMOTE_SYNC_INTERVAL") {
            insert_key_value(&mut layer, "remote-sync-interval", value);
        }
        if let Ok(value) = env::var("BEADS_AUTO_START_DAEMON") {
            if let Some(enabled) = parse_bool(&value) {
                insert_key_value(&mut layer, "no-daemon", (!enabled).to_string());
            }
        }

        layer
    }

    /// Build a layer from DB config table values.
    ///
    /// # Errors
    ///
    /// Returns an error if config table lookup fails.
    pub fn from_db(storage: &SqliteStorage) -> Result<Self> {
        let mut layer = Self::default();
        let map = storage.get_all_config()?;
        for (key, value) in map {
            if is_startup_key(&key) {
                continue;
            }
            layer.runtime.insert(key, value);
        }
        Ok(layer)
    }
}

/// CLI overrides for config loading (optional).
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub db: Option<PathBuf>,
    pub actor: Option<String>,
    pub identity: Option<String>,
    pub json: Option<bool>,
    pub no_db: Option<bool>,
    pub no_daemon: Option<bool>,
    pub no_auto_flush: Option<bool>,
    pub no_auto_import: Option<bool>,
    pub lock_timeout: Option<u64>,
}

impl CliOverrides {
    #[must_use]
    pub fn as_layer(&self) -> ConfigLayer {
        let mut layer = ConfigLayer::default();

        if let Some(path) = &self.db {
            insert_key_value(&mut layer, "db", path.to_string_lossy().to_string());
        }
        if let Some(actor) = &self.actor {
            insert_key_value(&mut layer, "actor", actor.clone());
        }
        if let Some(identity) = &self.identity {
            insert_key_value(&mut layer, "identity", identity.clone());
        }
        if let Some(json) = self.json {
            insert_key_value(&mut layer, "json", json.to_string());
        }
        if let Some(no_db) = self.no_db {
            insert_key_value(&mut layer, "no-db", no_db.to_string());
        }
        if let Some(no_daemon) = self.no_daemon {
            insert_key_value(&mut layer, "no-daemon", no_daemon.to_string());
        }
        if let Some(no_auto_flush) = self.no_auto_flush {
            insert_key_value(&mut layer, "no-auto-flush", no_auto_flush.to_string());
        }
        if let Some(no_auto_import) = self.no_auto_import {
            insert_key_value(&mut layer, "no-auto-import", no_auto_import.to_string());
        }
        if let Some(lock_timeout) = self.lock_timeout {
            insert_key_value(&mut layer, "lock-timeout", lock_timeout.to_string());
        }

        layer
    }
}

/// Load project config (.beads/config.yaml).
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read or parsed.
pub fn load_project_config(beads_dir: &Path) -> Result<ConfigLayer> {
    ConfigLayer::from_yaml(&beads_dir.join("config.yaml"))
}

/// Load user config (~/.config/bd/config.yaml).
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read or parsed.
pub fn load_user_config() -> Result<ConfigLayer> {
    let home = env::var("HOME").map_err(|_| BeadsError::Config("HOME not set".to_string()))?;
    let path = Path::new(&home)
        .join(".config")
        .join("bd")
        .join("config.yaml");
    ConfigLayer::from_yaml(&path)
}

/// Load legacy user config (~/.beads/config.yaml).
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read or parsed.
pub fn load_legacy_user_config() -> Result<ConfigLayer> {
    let home = env::var("HOME").map_err(|_| BeadsError::Config("HOME not set".to_string()))?;
    let path = Path::new(&home).join(".beads").join("config.yaml");
    ConfigLayer::from_yaml(&path)
}

/// Default config layer (lowest precedence).
#[must_use]
pub fn default_config_layer() -> ConfigLayer {
    let mut layer = ConfigLayer::default();
    layer
        .runtime
        .insert("issue_prefix".to_string(), "bd".to_string());
    layer
}

/// Load configuration with classic precedence order.
///
/// # Errors
///
/// Returns an error if any config file cannot be read or parsed, or DB access fails.
pub fn load_config(
    beads_dir: &Path,
    storage: Option<&SqliteStorage>,
    cli: &CliOverrides,
) -> Result<ConfigLayer> {
    let defaults = default_config_layer();
    let db_layer = match storage {
        Some(storage) => ConfigLayer::from_db(storage)?,
        None => ConfigLayer::default(),
    };
    let legacy_user = load_legacy_user_config()?;
    let user = load_user_config()?;
    let project = load_project_config(beads_dir)?;
    let env_layer = ConfigLayer::from_env();
    let cli_layer = cli.as_layer();

    Ok(ConfigLayer::merge_layers(&[
        defaults,
        db_layer,
        legacy_user,
        user,
        project,
        env_layer,
        cli_layer,
    ]))
}

/// Build ID generation config from a merged config layer.
#[must_use]
pub fn id_config_from_layer(layer: &ConfigLayer) -> IdConfig {
    let prefix = get_value(layer, &["issue_prefix", "issue-prefix"])
        .cloned()
        .unwrap_or_else(|| "bd".to_string());

    let min_hash_length = parse_usize(layer, &["min_hash_length", "min-hash-length"]).unwrap_or(3);
    let max_hash_length = parse_usize(layer, &["max_hash_length", "max-hash-length"]).unwrap_or(8);
    let max_collision_prob =
        parse_f64(layer, &["max_collision_prob", "max-collision-prob"]).unwrap_or(0.25);

    IdConfig {
        prefix,
        min_hash_length,
        max_hash_length,
        max_collision_prob,
    }
}

/// Determine if a key is startup-only.
fn is_startup_key(key: &str) -> bool {
    let normalized = normalize_key(key);

    if normalized.starts_with("git.")
        || normalized.starts_with("routing.")
        || normalized.starts_with("validation.")
        || normalized.starts_with("directory.")
    {
        return true;
    }

    matches!(
        normalized.as_str(),
        "no-db"
            | "no-daemon"
            | "no-auto-flush"
            | "no-auto-import"
            | "json"
            | "db"
            | "actor"
            | "identity"
            | "flush-debounce"
            | "lock-timeout"
            | "remote-sync-interval"
            | "no-git-ops"
            | "no-push"
            | "sync-branch"
            | "sync.branch"
            | "external-projects"
            | "hierarchy.max-depth"
    )
}

fn insert_key_value(layer: &mut ConfigLayer, key: &str, value: String) {
    if is_startup_key(key) {
        layer.startup.insert(key.to_string(), value);
    } else {
        layer.runtime.insert(key.to_string(), value);
    }
}

fn normalize_key(key: &str) -> String {
    key.trim().to_lowercase().replace('_', "-")
}

fn env_key_variants(raw: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let raw_lower = raw.to_lowercase();
    variants.push(raw_lower.clone());
    variants.push(raw_lower.replace('_', "."));
    variants.push(raw_lower.replace('_', "-"));
    variants
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn get_value<'a>(layer: &'a ConfigLayer, keys: &[&str]) -> Option<&'a String> {
    for key in keys {
        if let Some(value) = layer.runtime.get(*key) {
            return Some(value);
        }
    }
    None
}

fn parse_usize(layer: &ConfigLayer, keys: &[&str]) -> Option<usize> {
    get_value(layer, keys).and_then(|value| value.trim().parse::<usize>().ok())
}

fn parse_f64(layer: &ConfigLayer, keys: &[&str]) -> Option<f64> {
    get_value(layer, keys).and_then(|value| value.trim().parse::<f64>().ok())
}

fn layer_from_yaml_value(value: &serde_yaml::Value) -> ConfigLayer {
    let mut layer = ConfigLayer::default();
    let mut flat = HashMap::new();
    flatten_yaml(value, "", &mut flat);

    for (key, value) in flat {
        insert_key_value(&mut layer, &key, value);
    }

    layer
}

fn flatten_yaml(value: &serde_yaml::Value, prefix: &str, out: &mut HashMap<String, String>) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, value) in map {
                let Some(key_str) = key.as_str() else {
                    continue;
                };
                let next_prefix = if prefix.is_empty() {
                    key_str.to_string()
                } else {
                    format!("{prefix}.{key_str}")
                };
                flatten_yaml(value, &next_prefix, out);
            }
        }
        serde_yaml::Value::Sequence(values) => {
            let joined = values
                .iter()
                .filter_map(yaml_scalar_to_string)
                .collect::<Vec<_>>()
                .join(",");
            out.insert(prefix.to_string(), joined);
        }
        _ => {
            if let Some(value) = yaml_scalar_to_string(value) {
                out.insert(prefix.to_string(), value);
            }
        }
    }
}

fn yaml_scalar_to_string(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::Bool(v) => Some(v.to_string()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Null
        | serde_yaml::Value::Sequence(_)
        | serde_yaml::Value::Mapping(_) => None,
        serde_yaml::Value::Tagged(tagged) => yaml_scalar_to_string(&tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteStorage;
    use tempfile::TempDir;

    #[test]
    fn metadata_defaults_when_missing() {
        let temp = TempDir::new().expect("tempdir");
        let beads_dir = temp.path().join(".beads");
        fs::create_dir_all(&beads_dir).expect("create beads dir");

        let metadata = Metadata::load(&beads_dir).expect("metadata");
        assert_eq!(metadata.database, DEFAULT_DB_FILENAME);
        assert_eq!(metadata.jsonl_export, DEFAULT_JSONL_FILENAME);
    }

    #[test]
    fn metadata_override_paths() {
        let temp = TempDir::new().expect("tempdir");
        let beads_dir = temp.path().join(".beads");
        fs::create_dir_all(&beads_dir).expect("create beads dir");

        let metadata_path = beads_dir.join("metadata.json");
        let metadata =
            r#"{\n  \"database\": \"custom.db\",\n  \"jsonl_export\": \"custom.jsonl\"\n}"#;
        fs::write(metadata_path, metadata).expect("write metadata");

        let paths = ConfigPaths::resolve(&beads_dir, None).expect("paths");
        assert_eq!(paths.db_path, beads_dir.join("custom.db"));
        assert_eq!(paths.jsonl_path, beads_dir.join("custom.jsonl"));
    }

    #[test]
    fn merge_precedence_order() {
        let mut defaults = default_config_layer();
        defaults
            .runtime
            .insert("issue_prefix".to_string(), "bd".to_string());

        let mut db = ConfigLayer::default();
        db.runtime
            .insert("issue_prefix".to_string(), "db".to_string());

        let mut yaml = ConfigLayer::default();
        yaml.runtime
            .insert("issue_prefix".to_string(), "yaml".to_string());

        let mut env_layer = ConfigLayer::default();
        env_layer
            .runtime
            .insert("issue_prefix".to_string(), "env".to_string());

        let mut cli = ConfigLayer::default();
        cli.runtime
            .insert("issue_prefix".to_string(), "cli".to_string());

        let merged = ConfigLayer::merge_layers(&[defaults, db, yaml, env_layer, cli]);
        assert_eq!(merged.runtime.get("issue_prefix").unwrap(), "cli");
    }

    #[test]
    fn yaml_startup_keys_are_separated() {
        let yaml = r"
no-db: true
issue_prefix: bd
";
        let value: serde_yaml::Value = serde_yaml::from_str(yaml).expect("parse yaml");
        let layer = layer_from_yaml_value(&value);
        assert_eq!(layer.startup.get("no-db").unwrap(), "true");
        assert_eq!(layer.runtime.get("issue_prefix").unwrap(), "bd");
    }

    #[test]
    fn db_layer_skips_startup_keys() {
        let mut storage = SqliteStorage::open_memory().expect("storage");
        storage.set_config("no-db", "true").expect("set no-db");
        storage
            .set_config("issue_prefix", "bd")
            .expect("set issue_prefix");

        let layer = ConfigLayer::from_db(&storage).expect("db layer");
        assert!(!layer.startup.contains_key("no-db"));
        assert_eq!(layer.runtime.get("issue_prefix").unwrap(), "bd");
    }

    #[test]
    fn discover_beads_dir_uses_env_override() {
        let temp = TempDir::new().expect("tempdir");
        let beads_dir = temp.path().join(".beads");
        fs::create_dir_all(&beads_dir).expect("create beads dir");

        let discovered = discover_beads_dir_with_env(None, Some(&beads_dir)).expect("discover");
        assert_eq!(discovered, beads_dir);
    }

    #[test]
    fn discover_beads_dir_walks_up() {
        let temp = TempDir::new().expect("tempdir");
        let beads_dir = temp.path().join(".beads");
        fs::create_dir_all(&beads_dir).expect("create beads dir");
        let nested = temp.path().join("a").join("b");
        fs::create_dir_all(&nested).expect("create nested");

        let discovered = discover_beads_dir(Some(&nested)).expect("discover");
        assert_eq!(discovered, beads_dir);
    }
}
