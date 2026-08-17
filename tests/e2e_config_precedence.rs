use std::fs;

mod common;

use common::cli::{ObrWorkspace, run_obr, run_obr_with_env};
use obr::storage::SqliteStorage;

#[test]
fn e2e_config_precedence_env_project_user_db() {
    let _log = common::test_log("e2e_config_precedence_env_project_user_db");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // DB layer (lowest non-default)
    let db_path = workspace.root.join(".obr").join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).expect("open db");
    storage
        .set_config("issue_prefix", "DB")
        .expect("set db issue_prefix");
    storage
        .set_config("default_priority", "1")
        .expect("set db default_priority");

    // User config layer (~/.config/obr/config.yaml)
    let user_config = workspace
        .root
        .join(".config")
        .join("obr")
        .join("config.yaml");
    fs::create_dir_all(user_config.parent().unwrap()).expect("create user config dir");
    fs::write(&user_config, "issue_prefix: USER\ndefault_priority: 2\n")
        .expect("write user config");

    // Project config layer (.obr/config.yaml)
    let project_config = workspace.root.join(".obr").join("config.yaml");
    fs::write(&project_config, "issue_prefix: PROJECT\n").expect("write project config");

    // No env: project wins for issue_prefix
    let get_project = run_obr(&workspace, ["config", "get", "issue_prefix"], "get_project");
    assert!(
        get_project.status.success(),
        "config get issue_prefix failed: {}",
        get_project.stderr
    );
    assert!(
        get_project.stdout.trim() == "PROJECT",
        "expected PROJECT, got stdout='{}', stderr='{}'",
        get_project.stdout,
        get_project.stderr
    );

    // No env: user wins over DB for default_priority (project doesn't set it)
    let get_user = run_obr(
        &workspace,
        ["config", "get", "default_priority"],
        "get_user",
    );
    assert!(
        get_user.status.success(),
        "config get default_priority failed: {}",
        get_user.stderr
    );
    assert!(
        get_user.stdout.trim() == "2",
        "expected default_priority=2 from user config, got stdout='{}'",
        get_user.stdout
    );

    // Env overrides project/user/DB
    let env_vars = vec![("OBR_ISSUE_PREFIX", "ENV")];
    let get_env = run_obr_with_env(
        &workspace,
        ["config", "get", "issue_prefix"],
        env_vars,
        "get_env",
    );
    assert!(
        get_env.status.success(),
        "config get with env failed: {}",
        get_env.stderr
    );
    assert!(
        get_env.stdout.trim() == "ENV",
        "expected ENV override, got stdout='{}'",
        get_env.stdout
    );
}

#[test]
fn e2e_config_precedence_cli_over_env_project() {
    let _log = common::test_log("e2e_config_precedence_cli_over_env_project");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    // Project config sets lock-timeout
    let project_config = workspace.root.join(".obr").join("config.yaml");
    fs::write(&project_config, "lock-timeout: 2500\n").expect("write project config");

    // Env overrides project
    let env_vars = vec![("OBR_LOCK_TIMEOUT", "3000")];
    let get_env = run_obr_with_env(
        &workspace,
        ["config", "get", "lock-timeout"],
        env_vars.clone(),
        "get_env_lock_timeout",
    );
    assert!(
        get_env.status.success(),
        "config get lock-timeout failed: {}",
        get_env.stderr
    );
    assert!(
        get_env.stdout.trim() == "3000",
        "expected env lock-timeout=3000, got stdout='{}'",
        get_env.stdout
    );

    // CLI overrides env + project
    let get_cli = run_obr_with_env(
        &workspace,
        ["--lock-timeout", "1234", "config", "get", "lock-timeout"],
        env_vars,
        "get_cli_lock_timeout",
    );
    assert!(
        get_cli.status.success(),
        "config get lock-timeout with CLI override failed: {}",
        get_cli.stderr
    );
    assert!(
        get_cli.stdout.trim() == "1234",
        "expected CLI lock-timeout=1234, got stdout='{}'",
        get_cli.stdout
    );
}

#[test]
fn e2e_config_get_ignores_global_no_db_flag() {
    let _log = common::test_log("e2e_config_get_ignores_global_no_db_flag");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let db_path = workspace.root.join(".obr").join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).expect("open db");
    storage
        .set_config("db_only_key", "from-db")
        .expect("set db-only config");

    let normal = run_obr(
        &workspace,
        ["config", "get", "db_only_key"],
        "get_db_only_key",
    );
    assert!(
        normal.status.success(),
        "config get db_only_key failed: {}",
        normal.stderr
    );
    assert_eq!(normal.stdout.trim(), "from-db");

    let no_db = run_obr(
        &workspace,
        ["--no-db", "config", "get", "db_only_key"],
        "get_db_only_key_no_db",
    );
    assert!(
        no_db.status.success(),
        "--no-db config get db_only_key failed: {}",
        no_db.stderr
    );
    assert_eq!(no_db.stdout.trim(), "from-db");
}

#[test]
fn e2e_config_set_preserves_malformed_project_yaml() {
    let _log = common::test_log("e2e_config_set_preserves_malformed_project_yaml");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let project_config = workspace.root.join(".obr").join("config.yaml");
    let malformed = "actor: [broken\n";
    fs::write(&project_config, malformed).expect("write malformed project config");

    let set = run_obr(
        &workspace,
        ["config", "set", "actor", "alice"],
        "set_malformed",
    );
    assert!(
        !set.status.success(),
        "config set should fail on malformed YAML"
    );
    assert!(
        set.stderr.contains("Failed to parse YAML config"),
        "unexpected stderr: {}",
        set.stderr
    );
    assert_eq!(
        fs::read_to_string(&project_config).expect("read config after failed set"),
        malformed
    );
}

#[test]
fn e2e_config_delete_preserves_db_when_project_yaml_is_malformed() {
    let _log = common::test_log("e2e_config_delete_preserves_db_when_project_yaml_is_malformed");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let db_path = workspace.root.join(".obr").join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).expect("open db");
    storage
        .set_config("db_only_key", "from-db")
        .expect("set db-only config");

    let project_config = workspace.root.join(".obr").join("config.yaml");
    let malformed = "actor: [broken\n";
    fs::write(&project_config, malformed).expect("write malformed project config");

    let delete = run_obr(
        &workspace,
        ["config", "delete", "db_only_key"],
        "delete_malformed_project_yaml",
    );
    assert!(
        !delete.status.success(),
        "config delete should fail on malformed YAML"
    );
    assert!(
        delete.stderr.contains("Failed to parse YAML config"),
        "unexpected stderr: {}",
        delete.stderr
    );
    assert_eq!(
        storage
            .get_config("db_only_key")
            .expect("read db config after failed delete")
            .as_deref(),
        Some("from-db")
    );
    assert_eq!(
        fs::read_to_string(&project_config).expect("read config after failed delete"),
        malformed
    );
}

#[test]
fn e2e_config_delete_no_db_preserves_db_values() {
    let _log = common::test_log("e2e_config_delete_no_db_preserves_db_values");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let db_path = workspace.root.join(".obr").join("obr.db");
    let mut storage = SqliteStorage::open(&db_path).expect("open db");
    storage
        .set_config("db_only_key", "from-db")
        .expect("set db-only config");

    let delete = run_obr(
        &workspace,
        ["--no-db", "config", "delete", "db_only_key", "--json"],
        "delete_db_only_key_no_db",
    );
    assert!(
        delete.status.success(),
        "--no-db config delete failed: {}",
        delete.stderr
    );

    let payload = common::cli::extract_json_payload(&delete.stdout);
    let result: serde_json::Value = serde_json::from_str(&payload).expect("delete json");
    assert_eq!(result["deleted_from_db"], false);

    let db_value = storage
        .get_config("db_only_key")
        .expect("read db config after no-db delete");
    assert_eq!(db_value.as_deref(), Some("from-db"));
}

#[test]
fn e2e_config_list_rejects_project_and_user_together() {
    let _log = common::test_log("e2e_config_list_rejects_project_and_user_together");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let list = run_obr(
        &workspace,
        ["config", "list", "--project", "--user"],
        "config_list_conflict",
    );
    assert!(
        !list.status.success(),
        "config list should reject mutually exclusive flags"
    );
    assert!(
        list.stderr.contains("cannot be used with"),
        "unexpected stderr: {}",
        list.stderr
    );
}

#[test]
fn e2e_create_normalizes_runtime_issue_prefix_from_project_config() {
    let _log = common::test_log("e2e_create_normalizes_runtime_issue_prefix_from_project_config");
    let workspace = ObrWorkspace::new();

    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    let project_config = workspace.root.join(".obr").join("config.yaml");
    fs::write(&project_config, "issue_prefix: \"Project-Name!\"\n").expect("write project config");

    let create = run_obr(
        &workspace,
        ["create", "Normalize prefix"],
        "create_with_mixed_prefix",
    );
    assert!(create.status.success(), "create failed: {}", create.stderr);
    assert!(
        create.stdout.contains("Created project-name-"),
        "expected normalized lowercase prefix, got stdout='{}', stderr='{}'",
        create.stdout,
        create.stderr
    );
}
