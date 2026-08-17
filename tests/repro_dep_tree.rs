mod common;
use common::cli::{ObrWorkspace, run_obr};

#[test]
fn test_dep_tree_diamond_dependency_visibility() {
    let workspace = ObrWorkspace::new();

    // Initialize
    run_obr(&workspace, ["init"], "init");

    // Create issues A, B, C, D and capture their IDs.
    // Dependencies: A -> B, A -> C, B -> D, C -> D (diamond).
    // "dep add X Y" means X depends on Y.
    // "dep tree A" walks what A depends on, so D should appear under both B and C.
    let id_a = run_obr(&workspace, ["create", "A", "--silent"], "get_A")
        .stdout
        .trim()
        .to_string();
    let id_b = run_obr(&workspace, ["create", "B", "--silent"], "get_B")
        .stdout
        .trim()
        .to_string();
    let id_c = run_obr(&workspace, ["create", "C", "--silent"], "get_C")
        .stdout
        .trim()
        .to_string();
    let id_d = run_obr(&workspace, ["create", "D", "--silent"], "get_D")
        .stdout
        .trim()
        .to_string();

    run_obr(&workspace, ["dep", "add", &id_a, &id_b], "A->B");
    run_obr(&workspace, ["dep", "add", &id_a, &id_c], "A->C");
    run_obr(&workspace, ["dep", "add", &id_b, &id_d], "B->D");
    run_obr(&workspace, ["dep", "add", &id_c, &id_d], "C->D");

    // Run tree on A (the root dependency)
    let tree = run_obr(&workspace, ["dep", "tree", &id_a], "tree").stdout;
    println!("Tree Output:\n{tree}");

    // Check if A appears twice (diamond convergence point)
    assert_eq!(
        tree.match_indices(&id_d).count(),
        2,
        "Diamond dependency node D should appear twice in tree view"
    );
}
