use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn test_create_deps_colon_title() {
    let temp = TempDir::new().unwrap();
    let obr_dir = temp.path().join(".obr");

    // Initialize the beads directory
    let mut cmd = Command::cargo_bin("obr").unwrap();
    cmd.arg("init").env("OBR_DIR", &obr_dir).assert().success();

    let mut cmd = Command::cargo_bin("obr").unwrap();
    let create = cmd
        .arg("create")
        .arg("Task: With colon")
        .arg("--json")
        .env("OBR_DIR", &obr_dir)
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "create dependency target failed: {}",
        String::from_utf8_lossy(&create.stderr)
    );
    let created: Value = serde_json::from_slice(&create.stdout).expect("create target json");
    let blocker_id = created["id"].as_str().expect("created issue id");

    let file_path = temp.path().join("issues.md");
    std::fs::write(
        &file_path,
        r"
## My Task

### Dependencies
- Task: With colon
",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("obr").unwrap();
    let imported = cmd
        .arg("create")
        .arg("-f")
        .arg(&file_path)
        .arg("--json")
        .env("OBR_DIR", &obr_dir)
        .output()
        .unwrap();
    assert!(
        imported.status.success(),
        "markdown import failed: {}",
        String::from_utf8_lossy(&imported.stderr)
    );

    let issues: Vec<Value> = serde_json::from_slice(&imported.stdout).expect("import json");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["title"].as_str(), Some("My Task"));
    let dependencies = issues[0]["dependencies"]
        .as_array()
        .expect("dependencies array");
    assert!(
        dependencies.iter().any(|dep| {
            dep["depends_on_id"].as_str() == Some(blocker_id)
                && dep["type"].as_str() == Some("blocks")
        }),
        "title with colon should resolve to dependency {blocker_id}, got {dependencies:?}"
    );
}
