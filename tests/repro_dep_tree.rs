mod common;
use common::cli::{BrWorkspace, run_br};

#[test]
fn test_dep_tree_diamond_dependency_visibility() {
    let workspace = BrWorkspace::new();

    // Initialize
    run_br(&workspace, ["init"], "init");

    // Create issues A, B, C, D
    run_br(&workspace, ["create", "A"], "create_A");
    run_br(&workspace, ["create", "B"], "create_B");
    run_br(&workspace, ["create", "C"], "create_C");
    run_br(&workspace, ["create", "D"], "create_D");

    // Get IDs (assuming predictable order or parse them)
    // A=bd-1, B=bd-2, C=bd-3, D=bd-4
    // Wait, IDs are hash based if title is unique? Or sequential?
    // beads_rust uses sequential hints if db is empty?
    // Let's use `br list --json` to get IDs.
    // Actually, `br create` outputs "Created <id>: ...".
    // I can just rely on `br list` to find them.

    // Let's setup dependencies:
    // A depends on B (A -> B)
    // A depends on C (A -> C)
    // B depends on D (B -> D)
    // C depends on D (C -> D)
    // Dependency direction: Child depends on Parent.
    // "dep add X Y" means X depends on Y.
    // "dep tree" walks DEPENDENTS (what depends on X).
    // So if we tree D, we should see B and C (because they depend on D).
    // And A depends on B/C, so A should appear under B and C.

    // We can use titles to refer to them if we use `br list`.
    // Or just `br create A -q` to get ID.

    let id_a = run_br(&workspace, ["create", "A", "--silent"], "get_A")
        .stdout
        .trim()
        .to_string();
    let id_b = run_br(&workspace, ["create", "B", "--silent"], "get_B")
        .stdout
        .trim()
        .to_string();
    let id_c = run_br(&workspace, ["create", "C", "--silent"], "get_C")
        .stdout
        .trim()
        .to_string();
    let id_d = run_br(&workspace, ["create", "D", "--silent"], "get_D")
        .stdout
        .trim()
        .to_string();

    run_br(&workspace, ["dep", "add", &id_a, &id_b], "A->B");
    run_br(&workspace, ["dep", "add", &id_a, &id_c], "A->C");
    run_br(&workspace, ["dep", "add", &id_b, &id_d], "B->D");
    run_br(&workspace, ["dep", "add", &id_c, &id_d], "C->D");

    // Run tree on D (the root blocker)
    let tree = run_br(&workspace, ["dep", "tree", &id_d], "tree").stdout;
    println!("Tree Output:\n{tree}");

    // Check if A appears twice (diamond convergence point)
    assert_eq!(
        tree.match_indices(&id_a).count(),
        2,
        "Diamond dependency node A should appear twice in tree view"
    );
}
