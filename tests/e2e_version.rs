//! E2E tests for the version command.
//!
//! Tests the `obr version` command and its flags: --short, --json.
//! Part of beads_rust-1hof.

mod common;

use common::cli::{ObrWorkspace, extract_json_payload, run_obr};
use serde_json::Value;

#[test]
fn e2e_version_short_flag() {
    let _log = common::test_log("e2e_version_short_flag");
    let workspace = ObrWorkspace::new();

    // Test --short flag
    let version = run_obr(&workspace, ["version", "--short"], "version_short");
    assert!(
        version.status.success(),
        "version --short failed: {}",
        version.stderr
    );

    let stdout = version.stdout.trim();

    // `--short` prints the version and nothing else. Assert that against the
    // semver grammar rather than a character whitelist: obr's own version is
    // `0.2.22+1` (the tracked upstream release plus the fork generation
    // since it), and a digits-and-dots whitelist rejects the `+` — as well as
    // any `-rc.1` this project might cut — while still accepting nonsense like
    // `....`.
    if let Err(error) = semver::Version::parse(stdout) {
        panic!("version --short must print a semver version, got '{stdout}': {error}");
    }

    // And it must be *this* build's version, not merely some version. Compared
    // against the crate version so the assertion carries no literal to bump.
    assert_eq!(
        stdout,
        env!("CARGO_PKG_VERSION"),
        "version --short must report the crate version"
    );
}

#[test]
fn e2e_version_json_flag() {
    let _log = common::test_log("e2e_version_json_flag");
    let workspace = ObrWorkspace::new();

    // Test --json flag
    let version = run_obr(&workspace, ["version", "--json"], "version_json");
    assert!(
        version.status.success(),
        "version --json failed: {}",
        version.stderr
    );

    let payload = extract_json_payload(&version.stdout);
    let json: Value = serde_json::from_str(&payload).expect("valid JSON");

    // Verify fields
    assert!(json.get("version").is_some(), "missing version field");
    assert!(json.get("build").is_some(), "missing build field");
    if option_env!("VERGEN_GIT_SHA").is_some() {
        assert!(json.get("commit").is_some(), "missing commit field");
    }
    // `mcp` is the only optional feature left, and an empty feature list is
    // skipped during serialization, so the field is present only under --features mcp.
    #[cfg(feature = "mcp")]
    assert!(json.get("features").is_some(), "missing features field");
    #[cfg(not(feature = "mcp"))]
    assert!(
        json.get("features").is_none(),
        "features field should be absent in a default build"
    );
}
