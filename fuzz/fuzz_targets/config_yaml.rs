#![no_main]

use obr::config::{self, ConfigLayer};
use obr::util::id::MAX_ID_HASH_LEN;
use libfuzzer_sys::fuzz_target;
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};

const MAX_INPUT_BYTES: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT_BYTES {
        return;
    }

    let result = run_config_yaml_case(data);
    assert!(
        result.is_ok(),
        "config YAML fuzz invariant failed: {result:?}"
    );
});

fn run_config_yaml_case(data: &[u8]) -> Result<(), Box<dyn Error>> {
    let config_path = fuzz_config_path();
    std::fs::write(&config_path, data)?;

    match ConfigLayer::from_yaml(&config_path) {
        Ok(layer) => validate_config_layer(&layer)?,
        Err(err) => assert_non_empty_error(&err.to_string(), "ConfigLayer::from_yaml")?,
    }

    Ok(())
}

fn fuzz_config_path() -> PathBuf {
    std::env::temp_dir().join(format!("br-config-yaml-fuzz-{}.yaml", std::process::id()))
}

fn validate_config_layer(layer: &ConfigLayer) -> Result<(), Box<dyn Error>> {
    validate_config_entries("startup", &layer.startup)?;
    validate_config_entries("runtime", &layer.runtime)?;
    validate_config_accessors(layer)?;
    validate_external_project_paths(layer)?;
    Ok(())
}

fn validate_config_entries(
    layer_name: &str,
    entries: &HashMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    for (key, value) in entries {
        if key.contains('\0') {
            return Err(format!("{layer_name} config key contains NUL byte").into());
        }
        if path_like_key(key) && value.contains('\0') {
            return Err(format!("path-like config value for key {key:?} contains NUL byte").into());
        }
    }

    Ok(())
}

fn validate_config_accessors(layer: &ConfigLayer) -> Result<(), Box<dyn Error>> {
    let id_config = config::id_config_from_layer(layer);
    if id_config.prefix.trim().is_empty() {
        return Err("ID config prefix resolved to an empty value".into());
    }
    if id_config.prefix.contains('\0') {
        return Err("ID config prefix contains NUL byte".into());
    }
    if id_config.min_hash_length == 0
        || id_config.max_hash_length == 0
        || id_config.min_hash_length > id_config.max_hash_length
        || id_config.max_hash_length > MAX_ID_HASH_LEN
    {
        return Err(format!("ID hash length bounds are invalid: {id_config:?}").into());
    }
    if !id_config.max_collision_prob.is_finite()
        || !(0.0..=1.0).contains(&id_config.max_collision_prob)
    {
        return Err(format!("ID collision probability is invalid: {id_config:?}").into());
    }

    if let Err(err) = config::default_priority_from_layer(layer) {
        assert_non_empty_error(&err.to_string(), "default_priority_from_layer")?;
    }
    if let Err(err) = config::default_issue_type_from_layer(layer) {
        assert_non_empty_error(&err.to_string(), "default_issue_type_from_layer")?;
    }

    let actor = config::resolve_actor(layer);
    if actor.trim().is_empty() {
        return Err("resolved actor is empty".into());
    }
    if actor.contains('\0') {
        return Err("resolved actor contains NUL byte".into());
    }

    let _ = config::display_color_from_layer(layer);
    let _ = config::claim_exclusive_from_layer(layer);
    let _ = config::lock_timeout_from_layer(layer);

    Ok(())
}

fn validate_external_project_paths(layer: &ConfigLayer) -> Result<(), Box<dyn Error>> {
    let obr_dir = Path::new("/tmp/br-config-yaml-fuzz/.beads");
    for (name, path) in config::external_projects_from_layer(layer, obr_dir) {
        if name.trim().is_empty() {
            return Err("external project name resolved to an empty value".into());
        }
        if name.contains('\0') {
            return Err("external project name contains NUL byte".into());
        }
        if path.to_string_lossy().contains('\0') {
            return Err(format!("external project path contains NUL byte: {path:?}").into());
        }
    }

    Ok(())
}

fn path_like_key(key: &str) -> bool {
    let normalized = key.trim().to_lowercase().replace('_', "-");
    normalized == "db"
        || normalized == "database"
        || normalized.ends_with("-path")
        || normalized.ends_with(".path")
        || normalized.ends_with("-dir")
        || normalized.ends_with(".dir")
        || normalized.starts_with("external-projects.")
}

fn assert_non_empty_error(message: &str, context: &str) -> Result<(), Box<dyn Error>> {
    if message.trim().is_empty() {
        return Err(format!("{context} returned an empty error message").into());
    }
    Ok(())
}
