use std::fs;
mod common;
use common::cli::{BrWorkspace, run_br};

#[test]
fn test_config_set_and_delete_yaml() {
    let workspace = BrWorkspace::new();
    
    // 0. Init workspace so DB exists
    let run_init = run_br(&workspace, ["init"], "init");
    assert!(run_init.status.success());

    // 1. Set a config value
    let run_set = run_br(&workspace, ["config", "--set", "repro_key=repro_value"], "set_config");
    assert!(run_set.status.success());
    assert!(run_set.stdout.contains("Set repro_key=repro_value"));

    // 2. Verify it's in the YAML file
    let config_path = workspace.root.join(".config/bd/config.yaml");
    assert!(config_path.exists());
    let content = fs::read_to_string(&config_path).expect("read config");
    assert!(content.contains("repro_key: repro_value"));

    // 3. Delete the config value
    let run_delete = run_br(&workspace, ["config", "--delete", "repro_key"], "delete_config");
    
    // 4. Verify delete ran successfully (it should find no key in DB but succeed)
    // Or it might say "Config key not found in database".
    println!("Delete output: {}", run_delete.stdout);
    println!("Delete stderr: {}", run_delete.stderr);
    
    // 5. Verify it's STILL in the YAML file (The Bug)
    let content_after = fs::read_to_string(&config_path).expect("read config");
    assert!(content_after.contains("repro_key: repro_value"), "Key should still exist in YAML because delete doesn't touch it");
}
