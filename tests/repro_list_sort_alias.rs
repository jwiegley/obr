mod common;
use common::cli::{ObrWorkspace, run_obr};

#[test]
fn test_list_sort_aliases_are_accepted() {
    let workspace = ObrWorkspace::new();
    let init = run_obr(&workspace, ["init"], "init");
    assert!(init.status.success(), "init failed: {}", init.stderr);

    for alias in ["created", "updated"] {
        let list = run_obr(
            &workspace,
            ["list", "--sort", alias, "--json"],
            &format!("list_sort_{alias}"),
        );
        assert!(
            list.status.success(),
            "list --sort {alias} failed: stdout={} stderr={}",
            list.stdout,
            list.stderr
        );
    }
}
