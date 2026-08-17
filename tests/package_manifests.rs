//! Package manifest validation tests.
//!
//! These tests validate the syntax and structure of package manager manifests
//! used for distributing obr through Homebrew, Scoop, and AUR.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Test that the Homebrew formula has valid Ruby syntax.
#[test]
fn test_homebrew_formula_syntax() {
    let formula_path = Path::new("packaging/homebrew/obr.rb");

    if !formula_path.exists() {
        eprintln!("Skipping: Homebrew formula not found at {formula_path:?}");
        return;
    }

    let content = fs::read_to_string(formula_path).expect("Failed to read Homebrew formula");

    // Basic structure checks
    assert!(
        content.contains("class Obr < Formula"),
        "Formula must define Obr class extending Formula"
    );
    assert!(
        content.contains("desc \""),
        "Formula must have a description"
    );
    assert!(
        content.contains("homepage \""),
        "Formula must have a homepage"
    );
    assert!(
        content.contains("license :cannot_represent"),
        "Formula must mark the rider-bearing license as non-SPDX"
    );
    assert!(
        content.contains("version \""),
        "Formula must have a version"
    );

    // Platform-specific URLs
    assert!(
        content.contains("on_macos do"),
        "Formula must have macOS platform section"
    );
    assert!(
        content.contains("on_linux do"),
        "Formula must have Linux platform section"
    );
    assert!(
        content.contains("on_arm do"),
        "Formula must have ARM architecture section"
    );
    assert!(
        content.contains("on_intel do"),
        "Formula must have Intel architecture section"
    );

    // Install and test blocks
    assert!(
        content.contains("def install"),
        "Formula must have install method"
    );
    assert!(content.contains("test do"), "Formula must have test block");

    // Check Ruby syntax if ruby is available
    if Command::new("ruby").arg("--version").output().is_ok() {
        let output = Command::new("ruby")
            .arg("-c")
            .arg(formula_path)
            .output()
            .expect("Failed to run ruby syntax check");

        assert!(
            output.status.success(),
            "Ruby syntax check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Test that the Scoop manifest has valid JSON schema.
#[test]
fn test_scoop_manifest_schema() {
    let manifest_path = Path::new("packaging/scoop/obr.json");

    if !manifest_path.exists() {
        eprintln!("Skipping: Scoop manifest not found at {manifest_path:?}");
        return;
    }

    let content = fs::read_to_string(manifest_path).expect("Failed to read Scoop manifest");

    // Parse as JSON
    let json: serde_json::Value =
        serde_json::from_str(&content).expect("Scoop manifest must be valid JSON");

    // Required fields
    assert!(
        json.get("version").is_some(),
        "Manifest must have 'version' field"
    );
    assert!(
        json.get("description").is_some(),
        "Manifest must have 'description' field"
    );
    assert!(
        json.get("homepage").is_some(),
        "Manifest must have 'homepage' field"
    );
    assert!(
        json.get("license").is_some(),
        "Manifest must have 'license' field"
    );
    assert_eq!(
        json.get("license").and_then(serde_json::Value::as_str),
        Some("https://github.com/Dicklesworthstone/beads_rust/blob/main/LICENSE"),
        "Scoop must link to the repository's rider-bearing license"
    );
    assert!(json.get("bin").is_some(), "Manifest must have 'bin' field");

    // Architecture section
    let arch = json
        .get("architecture")
        .expect("Manifest must have 'architecture' field");
    assert!(
        arch.get("64bit").is_some(),
        "Manifest must have '64bit' architecture"
    );

    // 64bit must have url and hash
    let arch_64 = arch.get("64bit").unwrap();
    assert!(
        arch_64.get("url").is_some(),
        "64bit architecture must have 'url'"
    );
    assert!(
        arch_64.get("hash").is_some(),
        "64bit architecture must have 'hash'"
    );

    // Autoupdate section (optional but recommended)
    if let Some(autoupdate) = json.get("autoupdate") {
        assert!(
            autoupdate.get("architecture").is_some(),
            "autoupdate must have 'architecture' section"
        );
    }

    // URL format validation
    let url = arch_64.get("url").unwrap().as_str().unwrap();
    assert!(url.starts_with("https://"), "URL must use HTTPS: {url}");
    assert!(
        url.contains("github.com"),
        "URL should point to GitHub releases"
    );
    // Allow case-sensitive comparison since URLs are case-sensitive
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    {
        assert!(url.ends_with(".zip"), "Windows URL should be a .zip file");
    }
}

/// Test that the AUR PKGBUILD has valid shell syntax.
#[test]
fn test_pkgbuild_syntax() {
    let pkgbuild_path = Path::new("packaging/aur/PKGBUILD");

    if !pkgbuild_path.exists() {
        eprintln!("Skipping: PKGBUILD not found at {pkgbuild_path:?}");
        return;
    }

    let content = fs::read_to_string(pkgbuild_path).expect("Failed to read PKGBUILD");

    // Required variables
    assert!(content.contains("pkgname="), "PKGBUILD must define pkgname");
    assert!(content.contains("pkgver="), "PKGBUILD must define pkgver");
    assert!(content.contains("pkgrel="), "PKGBUILD must define pkgrel");
    assert!(content.contains("pkgdesc="), "PKGBUILD must define pkgdesc");
    assert!(content.contains("arch="), "PKGBUILD must define arch");
    assert!(content.contains("url="), "PKGBUILD must define url");
    assert!(
        content.contains("license=('LicenseRef-MIT-OpenAI-Anthropic-Rider')"),
        "PKGBUILD must use a custom SPDX LicenseRef for the rider-bearing license"
    );

    // Source arrays for both architectures
    assert!(
        content.contains("source_x86_64="),
        "PKGBUILD must have x86_64 sources"
    );
    assert!(
        content.contains("source_aarch64="),
        "PKGBUILD must have aarch64 sources"
    );

    // SHA256 sums
    assert!(
        content.contains("sha256sums_x86_64="),
        "PKGBUILD must have x86_64 checksums"
    );
    assert!(
        content.contains("sha256sums_aarch64="),
        "PKGBUILD must have aarch64 checksums"
    );

    // Package function
    assert!(
        content.contains("package()"),
        "PKGBUILD must have package() function"
    );
    assert!(
        content.contains("/usr/share/licenses/${pkgname}/LICENSE"),
        "PKGBUILD must install the rider-bearing license text"
    );

    // Check bash syntax if bash is available
    if Command::new("bash").arg("--version").output().is_ok() {
        let output = Command::new("bash")
            .arg("-n")
            .arg(pkgbuild_path)
            .output()
            .expect("Failed to run bash syntax check");

        assert!(
            output.status.success(),
            "Bash syntax check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Test that Cargo.toml has proper metadata for crates.io publishing.
#[test]
fn test_cargo_metadata() {
    let cargo_toml = fs::read_to_string("Cargo.toml").expect("Failed to read Cargo.toml");
    let manifest: toml::Value = toml::from_str(&cargo_toml).expect("Cargo.toml must be valid TOML");
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .expect("Cargo.toml must have a package table");

    // Required fields for crates.io
    assert!(cargo_toml.contains("name = "), "Cargo.toml must have name");
    assert!(
        cargo_toml.contains("version = "),
        "Cargo.toml must have version"
    );
    assert!(
        cargo_toml.contains("description = "),
        "Cargo.toml must have description for crates.io"
    );
    assert_eq!(
        package.get("license-file").and_then(toml::Value::as_str),
        Some("LICENSE"),
        "Cargo.toml must publish the repository's nonstandard license text"
    );
    assert!(
        package.get("license").is_none(),
        "Cargo.toml must not mislabel the rider-bearing license as a standard SPDX license"
    );
    assert!(
        cargo_toml.contains("repository = "),
        "Cargo.toml should have repository URL"
    );

    // Recommended fields
    assert!(
        cargo_toml.contains("keywords = "),
        "Cargo.toml should have keywords for discoverability"
    );
    assert!(
        cargo_toml.contains("categories = "),
        "Cargo.toml should have categories for crates.io"
    );

    // Binary definition
    assert!(
        cargo_toml.contains("[[bin]]"),
        "Cargo.toml must define binary target"
    );
    assert!(
        cargo_toml.contains("name = \"obr\""),
        "Binary must be named 'obr'"
    );
}

/// Source-install instructions must be unambiguous and use the validated lockfile.
///
/// Without `--locked`, `cargo install` resolves a fresh dependency graph that
/// can require compiler features newer than the deliberately pinned nightly.
/// Git installs must also select `obr` because the repository contains
/// a second Cargo package for fuzzing.
#[test]
fn test_source_install_documentation_uses_locked_resolution() {
    for path in [
        "README.md",
        "docs/INSTALLING.md",
        "docs/AGENT_INTEGRATION.md",
        "docs/CLI_REFERENCE.md",
        "Cargo.toml",
        ".github/workflows/release.yml",
    ] {
        let content =
            fs::read_to_string(path).expect("Failed to read source-install documentation");
        let logical_lines = content.replace("\\\n", " ");

        for (line_index, line) in logical_lines.lines().enumerate() {
            let is_git_source_install = line.contains(
                "cargo install --git https://github.com/Dicklesworthstone/beads_rust.git",
            );
            let is_source_install =
                line.contains("cargo install --path .") || is_git_source_install;
            if is_source_install {
                assert!(
                    line.contains("--locked"),
                    "{path}:{} source-install command must include --locked: {line}",
                    line_index + 1
                );
            }
            if is_git_source_install {
                assert!(
                    line.contains(
                        "cargo install --git https://github.com/Dicklesworthstone/beads_rust.git beads_rust",
                    ),
                    "{path}:{} git source-install command must select the obr package: {line}",
                    line_index + 1
                );
            }
        }
    }
}

fn parse_version(raw: &str, source: &str) -> Result<semver::Version, String> {
    semver::Version::parse(raw.trim())
        .map_err(|err| format!("{source} version '{raw}' is not valid semver: {err}"))
}

/// The fork generation `N` from a `+N` build metadata tag.
///
/// The grammar is the whole of the build metadata and nothing else: obr
/// versions as `<upstream core>+<generation>`, so `0.2.22+7` yields `7`.
///
/// Anything that is not that grammar returns `None` rather than a default, and
/// [`not_ahead_of`] turns `None` into a reported error instead of a silent
/// pass. That matters for exactly the inputs a bad edit produces: an empty
/// build (`0.2.22`), a stale `obr.`-prefixed generation (`0.2.22+obr.1`), a
/// dotted metadata (`0.2.22+1.2`), a hyphenated identifier (`0.2.22+-1`), or a
/// value too large for `u64`. A parser that defaulted those to 0 would call
/// every manifest "behind Cargo.toml" and assert nothing.
fn fork_generation(version: &semver::Version) -> Option<u64> {
    let raw = version.build.as_str();
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

/// Whether `manifest` is at or behind `cargo`, comparing build metadata
/// explicitly rather than trusting any one comparator to order it.
fn not_ahead_of(
    manifest: &semver::Version,
    cargo: &semver::Version,
    source: &str,
) -> Result<(), String> {
    match manifest.cmp(cargo) {
        std::cmp::Ordering::Less => Ok(()),
        std::cmp::Ordering::Greater => Err(format!(
            "{source} version {manifest} is ahead of Cargo.toml {cargo}"
        )),
        // The `semver` crate's `Ord` compares build metadata too, so today
        // this arm is reached only when the two versions agree completely.
        // It is kept for the world where that stops being true — a
        // spec-faithful `Version`, or a different comparator — because then
        // `Greater` above would never fire on a fork-generation bump and this
        // is the only thing standing between a stale manifest and a green run.
        std::cmp::Ordering::Equal => {
            if manifest.build == cargo.build {
                return Ok(());
            }
            match (fork_generation(manifest), fork_generation(cargo)) {
                (Some(manifest_generation), Some(cargo_generation)) => {
                    if manifest_generation <= cargo_generation {
                        Ok(())
                    } else {
                        Err(format!(
                            "{source} fork generation +{} is ahead of Cargo.toml +{}",
                            manifest.build, cargo.build
                        ))
                    }
                }
                // A manifest still on a bare version is simply lagging.
                (None, _) if manifest.build.is_empty() => Ok(()),
                _ => Err(format!(
                    "{source} build metadata '+{}' cannot be ordered against Cargo.toml \
                     '+{}'; both must be the bare `<generation>` form or identical",
                    manifest.build, cargo.build
                )),
            }
        }
    }
}

/// The fork-generation grammar is `+<digits>` and nothing else.
///
/// [`fork_generation`] is the one place that reads meaning out of build
/// metadata, and its failure mode matters more than its success: a parser that
/// answered `Some(0)` for input it did not understand would make
/// [`not_ahead_of`] pass vacuously for every manifest. Pin both halves.
#[test]
fn fork_generation_parses_bare_digits_and_rejects_everything_else() {
    let generation = |raw: &str| fork_generation(&semver::Version::parse(raw).expect(raw));

    assert_eq!(generation("0.2.22+1"), Some(1));
    assert_eq!(generation("0.2.22+7"), Some(7));
    // Past the point where a lexical comparison would invert.
    assert_eq!(generation("0.2.22+10"), Some(10));
    assert_eq!(generation("0.2.22+999"), Some(999));

    // No build metadata at all: not a fork generation. `not_ahead_of` reads
    // this as "manifest is lagging", which is why it must be distinguishable.
    assert_eq!(generation("0.2.22"), None);
    // The superseded `+obr.N` spelling must not be silently accepted.
    assert_eq!(generation("0.2.22+obr.1"), None);
    // Dotted, alphabetic, hyphenated, and mixed identifiers are all legal
    // semver build metadata and all outside this grammar.
    assert_eq!(generation("0.2.22+1.2"), None);
    assert_eq!(generation("0.2.22+build"), None);
    assert_eq!(generation("0.2.22+1a"), None);
    assert_eq!(generation("0.2.22+-1"), None);
    // Numeric but unrepresentable: `None`, never a truncated or default value.
    assert_eq!(generation("0.2.22+99999999999999999999999999"), None);

    // And the pre-release tail is not the fork generation either.
    assert_eq!(generation("0.2.22-rc.1"), None);
    assert_eq!(generation("0.2.22-rc.1+4"), Some(4));
}

/// Test that all package manifests carry a version no newer than Cargo.toml.
///
/// The manually dispatched `update-package-manifests.yml` workflow rewrites
/// every checksummed packaging manifest after a release is published. During
/// development Cargo.toml is bumped ahead of manifests; this test only asserts
/// that manifest versions parse and are not ahead of Cargo.toml.
///
/// obr's version carries semver build metadata (`0.5.7+1`). [`not_ahead_of`]
/// explicitly breaks metadata ties so its result does not depend on whether a
/// version comparator orders build metadata.
#[test]
fn test_version_consistency() -> Result<(), String> {
    let cargo_toml = fs::read_to_string("Cargo.toml").expect("Failed to read Cargo.toml");
    let cargo_version_str = cargo_toml
        .lines()
        .find(|line| line.starts_with("version = "))
        .and_then(|line| line.split('"').nth(1))
        .expect("Could not find version in Cargo.toml");
    let cargo_version = parse_version(cargo_version_str, "Cargo.toml")?;

    let formula_path = Path::new("packaging/homebrew/obr.rb");
    if formula_path.exists() {
        let formula = fs::read_to_string(formula_path).expect("Failed to read Homebrew formula");
        let raw = formula
            .lines()
            .find_map(|line| {
                let line = line.trim();
                line.strip_prefix("version \"")
                    .and_then(|rest| rest.strip_suffix('"'))
            })
            .expect("Homebrew formula missing `version \"…\"` line");
        let manifest_version = parse_version(raw, "Homebrew formula")?;
        not_ahead_of(&manifest_version, &cargo_version, "Homebrew formula")?;
    }

    let scoop_path = Path::new("packaging/scoop/obr.json");
    if scoop_path.exists() {
        let scoop = fs::read_to_string(scoop_path).expect("Failed to read Scoop manifest");
        let scoop_json: serde_json::Value =
            serde_json::from_str(&scoop).expect("Invalid Scoop JSON");
        let scoop_version_str = scoop_json
            .get("version")
            .and_then(serde_json::Value::as_str)
            .expect("Scoop manifest missing `version`");
        let scoop_version = parse_version(scoop_version_str, "Scoop manifest")?;
        not_ahead_of(&scoop_version, &cargo_version, "Scoop manifest")?;
    }

    let pkgbuild_path = Path::new("packaging/aur/PKGBUILD");
    if pkgbuild_path.exists() {
        let pkgbuild = fs::read_to_string(pkgbuild_path).expect("Failed to read PKGBUILD");
        let raw = pkgbuild
            .lines()
            .find_map(|line| line.trim().strip_prefix("pkgver="))
            .expect("PKGBUILD missing `pkgver=` line");
        let manifest_version = parse_version(raw, "PKGBUILD")?;
        not_ahead_of(&manifest_version, &cargo_version, "PKGBUILD")?;

        // makepkg rejects exactly one class of character in pkgver: colons,
        // forward slashes, hyphens, whitespace, and non-ASCII (libmakepkg
        // `check_pkgver`). `+` is not in that set, which is what lets the fork
        // generation ride along in the AUR version; a pre-release `-rc.1`
        // would not be legal here.
        for forbidden in [':', '/', '-', ' ', '\t'] {
            assert!(
                !raw.contains(forbidden),
                "PKGBUILD pkgver '{raw}' contains {forbidden:?}, which makepkg rejects"
            );
        }
        assert!(
            raw.is_ascii(),
            "PKGBUILD pkgver '{raw}' must be ASCII; makepkg rejects anything else"
        );
    }

    Ok(())
}

/// Package-manager manifests must follow the artifact names that DSR publishes.
///
/// `obr-v<version>-...` would be the older GitHub Actions naming convention.
/// Releases publish installer-compatible archives as
/// `obr-<version>-<platform>...`, so a stale `obr-v...` URL makes
/// package-manager automation look for assets that do not exist on the
/// release.
#[test]
fn test_package_manifests_use_dsr_asset_names() {
    for path in [
        "packaging/homebrew/obr.rb",
        "packaging/scoop/obr.json",
        "packaging/aur/PKGBUILD",
    ] {
        let content = fs::read_to_string(path).expect("Failed to read package manifest");
        assert!(
            !content.contains("PLACEHOLDER_"),
            "{path} must not ship placeholder checksums"
        );
        assert!(
            !content.contains("obr-v"),
            "{path} must use obr-<version> asset names, not stale obr-v names"
        );
    }

    // Asset FILE names are built from the version with semver build metadata's
    // `+` flattened to `.`; the tag path segment keeps the version verbatim.
    // See `manifest_release_urls_split_the_tag_from_the_asset_name` for why,
    // and for the check that the two halves actually agree.
    let formula =
        fs::read_to_string("packaging/homebrew/obr.rb").expect("Failed to read Homebrew formula");
    for platform in ["darwin_arm64", "darwin_amd64", "linux_arm64", "linux_amd64"] {
        let expected = format!(r#"obr-#{{version.to_s.tr("+", ".")}}-{platform}.tar.gz"#);
        assert!(
            formula.contains(&expected),
            "Homebrew formula must build the {platform} asset name as {expected}"
        );
    }

    let scoop = fs::read_to_string("packaging/scoop/obr.json").expect("Failed to read Scoop file");
    assert!(scoop.contains("-windows_amd64.zip"));

    let pkgbuild = fs::read_to_string("packaging/aur/PKGBUILD").expect("Failed to read PKGBUILD");
    assert!(pkgbuild.contains(r#"_assetver="${pkgver//+/.}""#));
    assert!(pkgbuild.contains("obr-${_assetver}-linux_amd64.tar.gz"));
    assert!(pkgbuild.contains("obr-${_assetver}-linux_arm64.tar.gz"));
}

/// A release URL carries the version twice, spelled two different ways, and
/// the manifests must get both right.
///
/// The tag path segment is the version verbatim — `v0.2.22+1`. Git accepts
/// `+` in a refname and so does a URL path segment. The asset FILE name is the
/// same version with `+` flattened to `.` — `obr-0.2.22.1-…` — because
/// GitHub's release-asset upload API takes the asset name as a URL *query*
/// parameter, where a literal `+` decodes to a space and is stored back as
/// `.`. `.github/workflows/release.yml` therefore flattens it before uploading,
/// so a manifest that interpolated the raw version into the file name would
/// point at a file that was never published.
///
/// This is checked per manifest against that manifest's OWN declared version,
/// not against `Cargo.toml`, because the manifests are allowed to lag a release
/// behind (see `test_version_consistency`). What must never lag is a manifest's
/// URL relative to its own version.
#[test]
fn manifest_release_urls_split_the_tag_from_the_asset_name() -> Result<(), String> {
    const RELEASES: &str = "https://github.com/jwiegley/obr/releases/download";

    // Scoop's concrete URL is a literal — its schema forbids `$` there — so it
    // can be reconstructed exactly and compared.
    let scoop_path = Path::new("packaging/scoop/obr.json");
    if scoop_path.exists() {
        let scoop: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(scoop_path).expect("read Scoop manifest"))
                .expect("Invalid Scoop JSON");
        let version = scoop["version"].as_str().expect("Scoop version");
        let asset_version = version.replace('+', ".");
        let expected = format!("{RELEASES}/v{version}/obr-{asset_version}-windows_amd64.zip");
        let actual = scoop["architecture"]["64bit"]["url"]
            .as_str()
            .expect("Scoop 64bit url");
        if actual != expected {
            return Err(format!(
                "Scoop url is {actual:?}\n              expected {expected:?}"
            ));
        }
    }

    // The PKGBUILD is shell, so the rendered URLs can be obtained the only way
    // that proves anything: by letting bash render them.
    let pkgbuild_path = Path::new("packaging/aur/PKGBUILD");
    if pkgbuild_path.exists() && Command::new("bash").arg("--version").output().is_ok() {
        let output = Command::new("bash")
            .arg("-c")
            .arg(
                r#"set -euo pipefail
                   source packaging/aur/PKGBUILD
                   printf '%s\n' "$pkgver" "${source_x86_64[0]}" "${source_aarch64[0]}""#,
            )
            .output()
            .map_err(|error| format!("failed to source PKGBUILD: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "sourcing PKGBUILD failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut lines = stdout.lines();
        let pkgver = lines.next().ok_or("PKGBUILD produced no pkgver")?;
        let asset_version = pkgver.replace('+', ".");
        for (line, platform) in lines.zip(["linux_amd64", "linux_arm64"]) {
            let url = line.rsplit_once("::").map_or(line, |(_, url)| url);
            let expected = format!("{RELEASES}/v{pkgver}/obr-{asset_version}-{platform}.tar.gz");
            if url != expected {
                return Err(format!(
                    "PKGBUILD {platform} url is {url:?}\n                 expected {expected:?}"
                ));
            }
        }
    }

    // The Homebrew formula needs Homebrew's DSL to render, so assert the two
    // distinct expressions instead: tag from `version`, file name from the
    // flattened form. `test_package_manifests_use_dsr_asset_names` pins the
    // file-name half; this pins the tag half and that they are not the same.
    let formula_path = Path::new("packaging/homebrew/obr.rb");
    if formula_path.exists() {
        let formula = fs::read_to_string(formula_path).expect("read Homebrew formula");
        if !formula.contains(&format!("{RELEASES}/v#{{version}}/obr-")) {
            return Err(
                "Homebrew formula must build the tag path from the unflattened `version`"
                    .to_string(),
            );
        }
        #[allow(clippy::literal_string_with_formatting_args)]
        let raw_version_asset = "/obr-#{version}-";
        if formula.contains(raw_version_asset) {
            return Err(
                "Homebrew formula interpolates the raw version into an asset file name; \
                 GitHub does not publish the `+` form"
                    .to_string(),
            );
        }
    }

    Ok(())
}

#[test]
fn test_update_package_manifests_workflow_uses_current_checksums() {
    let workflow = fs::read_to_string(".github/workflows/update-package-manifests.yml")
        .expect("Failed to read update-package-manifests workflow");

    assert!(
        workflow.contains(r#"FILE="obr-${ASSET_VERSION}-${platform}.${ext}.sha256""#),
        "workflow must download the DSR-published checksum sidecars"
    );
    assert!(
        workflow.contains(r#"VERSION="${VERSION#v}""#),
        "workflow_dispatch inputs must normalize an optional leading v before building asset names"
    );
    // The sidecar names come from the flattened asset version, never the raw
    // one: release.yml publishes `obr-0.2.22.1-*.sha256`, so reconstructing
    // the `+` form here would 404 and abort the manifest update.
    assert!(
        workflow.contains(r#"ASSET_VERSION="${VERSION//+/.}""#),
        "workflow must flatten semver build metadata before building asset names"
    );
    assert!(
        !workflow.contains("obr-v${VERSION}"),
        "workflow must not look for stale obr-v checksum sidecars"
    );
    assert!(
        workflow.contains("curl -fsSL"),
        "checksum download must fail fast instead of saving a 404 body"
    );
    assert!(
        workflow.contains("Invalid SHA256"),
        "workflow must validate checksum file contents before updating manifests"
    );
}
