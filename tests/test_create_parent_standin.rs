use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn test_create_parent_standin() {
    let temp = TempDir::new().unwrap();
    let obr_dir = temp.path().join(".obr");

    // Initialize the beads directory
    let mut cmd = Command::cargo_bin("obr").unwrap();
    cmd.arg("init").env("OBR_DIR", &obr_dir).assert().success();

    let file_path = temp.path().join("issues.md");
    std::fs::write(
        &file_path,
        r"
## My Epic
### ID
epic1
### Type
epic

## My Task
### Parent
epic1
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
    assert_eq!(issues.len(), 2);

    let epic = issues
        .iter()
        .find(|issue| issue["title"].as_str() == Some("My Epic"))
        .expect("imported epic");
    assert_eq!(epic["issue_type"].as_str(), Some("epic"));
    let epic_id = epic["id"].as_str().expect("epic id");

    let task = issues
        .iter()
        .find(|issue| issue["title"].as_str() == Some("My Task"))
        .expect("imported task");
    let dependencies = task["dependencies"].as_array().expect("dependencies array");
    assert!(
        dependencies.iter().any(|dep| {
            dep["depends_on_id"].as_str() == Some(epic_id)
                && dep["type"].as_str() == Some("parent-child")
        }),
        "parent stand-in should resolve to epic {epic_id}, got {dependencies:?}"
    );
}

#[test]
fn test_create_parent_standin_forward_reference() {
    let temp = TempDir::new().unwrap();
    let obr_dir = temp.path().join(".obr");

    let mut cmd = Command::cargo_bin("obr").unwrap();
    cmd.arg("init").env("OBR_DIR", &obr_dir).assert().success();

    let file_path = temp.path().join("issues.md");
    std::fs::write(
        &file_path,
        r"
## My Task
### Parent
epic1

## My Epic
### ID
epic1
### Type
epic
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
    assert_eq!(issues.len(), 2);

    let epic = issues
        .iter()
        .find(|issue| issue["title"].as_str() == Some("My Epic"))
        .expect("imported epic");
    assert_eq!(epic["issue_type"].as_str(), Some("epic"));
    let epic_id = epic["id"].as_str().expect("epic id");

    let task = issues
        .iter()
        .find(|issue| issue["title"].as_str() == Some("My Task"))
        .expect("imported task");
    let dependencies = task["dependencies"].as_array().expect("dependencies array");
    assert!(
        dependencies.iter().any(|dep| {
            dep["depends_on_id"].as_str() == Some(epic_id)
                && dep["type"].as_str() == Some("parent-child")
        }),
        "forward parent stand-in should resolve to epic {epic_id}, got {dependencies:?}"
    );
}
