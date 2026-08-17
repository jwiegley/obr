//! Robot-docs command implementation.

use crate::cli::{
    OutputFormat, RobotDocsCommands, RobotDocsGuideArgs,
    resolve_output_format_basic_with_outer_mode,
};
use crate::error::Result;
use crate::output::{OutputContext, OutputMode};
use serde::Serialize;

const CONTRACT_VERSION: &str = "obr.robot_docs.v1";

const GUIDE: &str = r#"obr Agent Guide

Purpose:
  obr is a local-first issue tracker. It keeps a SQLite cache in .obr/ and
  exports the tracked PLAN.org surface for git-friendly handoff.
  obr never runs git on normal issue and sync paths.
  Only an explicit obr vcs-status request runs bounded, read-only Git probes.

Machine-output defaults:
  Use --json or --format json for scripts. Diagnostics and structured errors
  go to stderr. For token-efficient structured output, use --format toon where
  the command supports it.

Start of session:
  obr capabilities --format json
  obr ready --json
  obr coordination status --json
  obr show <id> --json

Finding work:
  obr ready --json is the single work-discovery entrypoint: it returns
  unblocked, non-deferred, actionable issues. "Ready" defaults to status=open,
  but projects can widen it via workflow.status_groups.ready in
  policy.yaml (e.g. [open, rework]) so review-returned work resurfaces
  without changing the command. Don't hand-roll status filters like
  `obr list -s open -s rework`; call `obr ready --json` and let project policy
  define readiness. Returned issues keep their real status (a rework item still
  reports {"status":"rework"}).

Claiming work:
  obr update <id> --claim --actor "$AGENT_NAME" --json
  If Agent Mail is down, add a comment naming the intended file scope before
  editing. Treat that comment as advisory, not a lock.

Completing work:
  obr close <id> --reason "Completed: <specific proof>" --json
  obr sync --flush-only
  Stage code and PLAN.org changes together outside obr.

Discovery:
  obr schema commands --format json
  obr schema all --format json
  obr vcs-status --json
  obr robot-docs guide

Safety:
  Avoid bare bv in automated sessions; use bv --robot-* flags.
  Use RUST_LOG=error for routine obr runs to suppress dependency logs.
  obr sync does not commit, push, pull, or install hooks.
  Existing databases are never schema-migrated implicitly. Run
  obr doctor migrate-schema plan --json, review the receipt, then apply its
  exact token.
"#;

#[derive(Debug, Serialize)]
struct RobotGuideOutput {
    tool: &'static str,
    version: &'static str,
    contract_version: &'static str,
    title: &'static str,
    line_count: usize,
    guide: &'static str,
    canonical_commands: &'static [CanonicalCommand],
}

#[derive(Debug, Serialize)]
struct CanonicalCommand {
    task: &'static str,
    command: &'static str,
}

const CANONICAL_COMMANDS: &[CanonicalCommand] = &[
    CanonicalCommand {
        task: "discover capabilities",
        command: "obr capabilities --format json",
    },
    CanonicalCommand {
        task: "find ready work",
        command: "obr ready --json",
    },
    CanonicalCommand {
        task: "diagnose stale claims",
        command: "obr coordination status --json",
    },
    CanonicalCommand {
        task: "show issue details",
        command: "obr show <id> --json",
    },
    CanonicalCommand {
        task: "inspect JSON contracts",
        command: "obr schema commands --format json",
    },
    CanonicalCommand {
        task: "explicitly inspect JSONL Git visibility",
        command: "obr vcs-status --json",
    },
    CanonicalCommand {
        task: "review a required schema migration",
        command: "obr doctor migrate-schema plan --json",
    },
    CanonicalCommand {
        task: "final JSONL export",
        command: "obr sync --flush-only",
    },
];

/// Execute the robot-docs command.
///
/// # Errors
///
/// Returns an error if output serialization fails.
pub fn execute(command: &RobotDocsCommands, outer_ctx: &OutputContext) -> Result<()> {
    match command {
        RobotDocsCommands::Guide(args) => execute_guide(args, outer_ctx),
    }
    Ok(())
}

fn execute_guide(args: &RobotDocsGuideArgs, outer_ctx: &OutputContext) {
    let output_format = resolve_output_format_basic_with_outer_mode(
        args.format,
        outer_ctx.inherited_output_mode(),
        false,
    );
    let quiet = matches!(outer_ctx.mode(), OutputMode::Quiet);
    let ctx = OutputContext::from_output_format(output_format, quiet, true);
    if ctx.is_quiet() {
        return;
    }

    let payload = RobotGuideOutput {
        tool: "obr",
        version: env!("CARGO_PKG_VERSION"),
        contract_version: CONTRACT_VERSION,
        title: "obr Agent Guide",
        line_count: GUIDE.lines().count(),
        guide: GUIDE,
        canonical_commands: CANONICAL_COMMANDS,
    };

    match output_format {
        OutputFormat::Json => ctx.json_pretty(&payload),
        OutputFormat::Toon => ctx.toon_with_stats(&payload, args.stats),
        OutputFormat::Text | OutputFormat::Csv => print!("{GUIDE}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{CANONICAL_COMMANDS, GUIDE};

    #[test]
    fn guide_scopes_git_authority_and_discovers_vcs_status() {
        assert!(GUIDE.contains("Only an explicit obr vcs-status request"));
        assert!(
            CANONICAL_COMMANDS
                .iter()
                .any(|entry| entry.command == "obr vcs-status --json")
        );
    }
}
