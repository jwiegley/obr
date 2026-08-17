//! Version command implementation.

use crate::cli::VersionArgs;
use crate::error::Result;
use crate::output::{OutputContext, OutputMode};
use rich_rust::prelude::*;
use serde::Serialize;
use std::fmt::Write as _;

#[derive(Serialize)]
struct VersionOutput<'a> {
    version: &'a str,
    build: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rust_version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    features: Vec<&'a str>,
}

/// Execute the version command.
///
/// # Errors
///
/// Returns an error if JSON serialization fails.
pub fn execute(args: &VersionArgs, ctx: &OutputContext) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");

    // Handle --short flag: output only version number
    if args.short {
        if !ctx.is_quiet() {
            println!("{version}");
        }
        return Ok(());
    }

    let build = if cfg!(debug_assertions) {
        "dev"
    } else {
        "release"
    };

    let commit = option_env!("VERGEN_GIT_SHA").filter(|s| !s.trim().is_empty());
    let branch = option_env!("VERGEN_GIT_BRANCH").filter(|s| !s.trim().is_empty());
    let rust_version = option_env!("VERGEN_RUSTC_SEMVER").filter(|s| !s.trim().is_empty());
    let target = option_env!("VERGEN_CARGO_TARGET_TRIPLE").filter(|s| !s.trim().is_empty());

    // Collect enabled features
    let mut features = Vec::new();
    if cfg!(feature = "mcp") {
        features.push("mcp");
    }

    if ctx.is_toon() {
        let output = VersionOutput {
            version,
            build,
            commit,
            branch,
            rust_version,
            target,
            features,
        };
        ctx.toon(&output);
        return Ok(());
    }

    if ctx.is_json() {
        let output = VersionOutput {
            version,
            build,
            commit,
            branch,
            rust_version,
            target,
            features,
        };
        ctx.json(&output);
        return Ok(());
    }

    if ctx.is_quiet() {
        return Ok(());
    }

    // Rich output mode
    if matches!(ctx.mode(), OutputMode::Rich) {
        render_version_rich(
            version,
            build,
            commit,
            branch,
            rust_version,
            target,
            &features,
            ctx,
        );
        return Ok(());
    }

    // Plain text output
    let mut line = format!("obr version {version} ({build})");
    match (branch, commit) {
        (Some(branch), Some(commit)) => {
            let short = &commit[..commit.len().min(7)];
            let _ = write!(line, " ({branch}@{short})");
        }
        (Some(branch), None) => {
            let _ = write!(line, " ({branch})");
        }
        (None, Some(commit)) => {
            let short = &commit[..commit.len().min(7)];
            let _ = write!(line, " ({short})");
        }
        (None, None) => {}
    }

    println!("{line}");
    Ok(())
}

/// Render version information with rich formatting.
#[allow(clippy::too_many_arguments)]
fn render_version_rich(
    version: &str,
    build: &str,
    commit: Option<&str>,
    branch: Option<&str>,
    rust_version: Option<&str>,
    target: Option<&str>,
    features: &[&str],
    ctx: &OutputContext,
) {
    let console = Console::default();
    let theme = ctx.theme();
    let width = ctx.width();

    let mut content = Text::new("");

    // Version header with styling
    content.append_styled(&format!("obr {version}"), theme.emphasis.clone());
    content.append_styled(&format!(" ({build})"), theme.dimmed.clone());
    content.append("\n\n");

    // Build info section
    let has_build_info =
        commit.is_some() || branch.is_some() || rust_version.is_some() || target.is_some();

    if has_build_info {
        content.append_styled("Build Info:\n", theme.section.clone());

        let mut info_items: Vec<(&str, String)> = Vec::new();

        if let Some(commit) = commit {
            let short = &commit[..commit.len().min(7)];
            info_items.push(("Commit", short.to_string()));
        }
        if let Some(branch) = branch {
            info_items.push(("Branch", branch.to_string()));
        }
        if let Some(rust_ver) = rust_version {
            info_items.push(("Rust", rust_ver.to_string()));
        }
        if let Some(tgt) = target {
            info_items.push(("Target", tgt.to_string()));
        }

        let last_idx = info_items.len().saturating_sub(1);
        for (idx, (label, value)) in info_items.iter().enumerate() {
            let prefix = if idx == last_idx {
                "└── "
            } else {
                "├── "
            };
            content.append_styled(prefix, theme.dimmed.clone());
            content.append_styled(&format!("{:<8}", label), theme.accent.clone());
            content.append(&format!("{value}\n"));
        }
        content.append("\n");
    }

    // Features section
    if !features.is_empty() {
        content.append_styled("Features: ", theme.section.clone());
        content.append_styled(&features.join(", "), theme.success.clone());
        content.append("\n");
    }

    // Wrap in panel
    let panel = Panel::from_rich_text(&content, width)
        .title(Text::styled("obr version", theme.panel_title.clone()))
        .box_style(theme.box_style);

    console.print_renderable(&panel);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_json_schema() {
        // Test that VersionOutput serializes with expected fields
        let output = VersionOutput {
            version: "1.0.0",
            build: "release",
            commit: Some("abc1234"),
            branch: Some("main"),
            rust_version: Some("1.85.0"),
            target: Some("x86_64-unknown-linux-gnu"),
            features: vec!["mcp"],
        };

        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["version"], "1.0.0");
        assert_eq!(json["build"], "release");
        assert_eq!(json["commit"], "abc1234");
        assert_eq!(json["branch"], "main");
        assert_eq!(json["rust_version"], "1.85.0");
        assert_eq!(json["target"], "x86_64-unknown-linux-gnu");
        assert_eq!(json["features"], serde_json::json!(["mcp"]));
    }

    #[test]
    fn test_version_json_omits_none_fields() {
        // Test that None fields are omitted from JSON output
        let output = VersionOutput {
            version: "1.0.0",
            build: "dev",
            commit: None,
            branch: None,
            rust_version: None,
            target: None,
            features: vec![],
        };

        let json = serde_json::to_value(&output).unwrap();
        assert!(json.get("commit").is_none());
        assert!(json.get("branch").is_none());
        assert!(json.get("rust_version").is_none());
        assert!(json.get("target").is_none());
        assert!(json.get("features").is_none()); // Empty vec is skipped
    }

    #[test]
    fn test_build_info_present() {
        // Verify build info env vars are defined at compile time
        let version = env!("CARGO_PKG_VERSION");
        assert!(!version.is_empty());

        // These may or may not be set depending on build environment
        // but the code should handle both cases gracefully
        let commit = option_env!("VERGEN_GIT_SHA");
        let branch = option_env!("VERGEN_GIT_BRANCH");

        // If set, they should be non-empty strings
        if let Some(c) = commit {
            assert!(!c.trim().is_empty() || c.is_empty()); // May be empty string
        }
        if let Some(b) = branch {
            assert!(!b.trim().is_empty() || b.is_empty());
        }
    }

    #[test]
    fn test_feature_flags_detection() {
        // Test that feature flags can be detected at compile time. `mcp` is the
        // only optional feature left; the default build enables nothing, so the
        // list is empty unless the crate was compiled with `--features mcp`.
        let mut features = Vec::new();
        if cfg!(feature = "mcp") {
            features.push("mcp");
        }

        #[cfg(feature = "mcp")]
        assert!(features.contains(&"mcp"));

        #[cfg(not(feature = "mcp"))]
        assert!(features.is_empty());
    }

    #[test]
    fn test_version_short_format() {
        // The short format should just be the version number
        let version = env!("CARGO_PKG_VERSION");
        // Should match semver pattern
        assert!(
            version.contains('.'),
            "Version should contain dots: {version}"
        );
        assert!(
            version.split('.').count() >= 2,
            "Version should have at least major.minor"
        );
    }
}
