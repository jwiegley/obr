//! Comments command implementation.

use crate::cli::{CommentAddArgs, CommentCommands, CommentListArgs, CommentsArgs};
use crate::config;
use crate::error::{BeadsError, Result};
use crate::storage::SqliteStorage;
use crate::util::id::{IdResolver, ResolverConfig, find_matching_ids};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Execute the comments command.
///
/// # Errors
///
/// Returns an error if database operations fail or if inputs are invalid.
pub fn execute(args: &CommentsArgs, actor: Option<&str>, json: bool) -> Result<()> {
    let beads_dir = config::discover_beads_dir(Some(Path::new(".")))?;
    let (mut storage, _paths) = config::open_storage(&beads_dir, None)?;

    let config_layer =
        config::load_config(&beads_dir, Some(&storage), &config::CliOverrides::default())?;
    let id_config = config::id_config_from_layer(&config_layer);
    let resolver = IdResolver::new(ResolverConfig::with_prefix(id_config.prefix));
    let all_ids = storage.get_all_ids()?;

    match &args.command {
        Some(CommentCommands::Add(add_args)) => {
            add_comment(add_args, &mut storage, &resolver, &all_ids, actor, json)
        }
        Some(CommentCommands::List(list_args)) => {
            list_comments(list_args, &storage, &resolver, &all_ids, json)
        }
        None => {
            let id = args
                .id
                .as_deref()
                .ok_or_else(|| BeadsError::validation("id", "missing issue id"))?;
            list_comments_by_id(id, &storage, &resolver, &all_ids, json)
        }
    }
}

fn add_comment(
    args: &CommentAddArgs,
    storage: &mut SqliteStorage,
    resolver: &IdResolver,
    all_ids: &[String],
    actor: Option<&str>,
    json: bool,
) -> Result<()> {
    let issue_id = resolve_issue_id(storage, resolver, all_ids, &args.id)?;
    let text = read_comment_text(args)?;
    if text.trim().is_empty() {
        return Err(BeadsError::validation(
            "text",
            "comment text cannot be empty",
        ));
    }
    let author = resolve_author(args.author.as_deref(), actor);

    let comment = storage.add_comment(&issue_id, &author, &text)?;

    if json {
        let output = serde_json::to_string_pretty(&comment)?;
        println!("{output}");
    } else {
        println!("Comment added to {issue_id}");
    }

    Ok(())
}

fn list_comments(
    args: &CommentListArgs,
    storage: &SqliteStorage,
    resolver: &IdResolver,
    all_ids: &[String],
    json: bool,
) -> Result<()> {
    list_comments_by_id(&args.id, storage, resolver, all_ids, json)
}

fn list_comments_by_id(
    id: &str,
    storage: &SqliteStorage,
    resolver: &IdResolver,
    all_ids: &[String],
    json: bool,
) -> Result<()> {
    let issue_id = resolve_issue_id(storage, resolver, all_ids, id)?;
    let comments = storage.get_comments(&issue_id)?;

    if json {
        let output = serde_json::to_string_pretty(&comments)?;
        println!("{output}");
        return Ok(());
    }

    if comments.is_empty() {
        println!("No comments for {issue_id}.");
        return Ok(());
    }

    println!("Comments for {issue_id}:");
    for comment in comments {
        let timestamp = comment.created_at.format("%Y-%m-%d %H:%M");
        println!("[{}] at {}", comment.author, timestamp);
        println!("{}", comment.body.trim_end_matches('\n'));
        println!();
    }

    Ok(())
}

fn resolve_issue_id(
    storage: &SqliteStorage,
    resolver: &IdResolver,
    all_ids: &[String],
    input: &str,
) -> Result<String> {
    resolver
        .resolve(
            input,
            |id| storage.id_exists(id).unwrap_or(false),
            |hash| find_matching_ids(all_ids, hash),
        )
        .map(|resolved| resolved.id)
}

fn read_comment_text(args: &CommentAddArgs) -> Result<String> {
    if let Some(path) = &args.file {
        return Ok(fs::read_to_string(path)?);
    }
    if let Some(message) = &args.message {
        return Ok(message.clone());
    }
    if !args.text.is_empty() {
        return Ok(args.text.join(" "));
    }
    Err(BeadsError::validation("text", "comment text required"))
}

fn resolve_author(author_override: Option<&str>, actor: Option<&str>) -> String {
    if let Some(author) = author_override {
        if !author.trim().is_empty() {
            return author.to_string();
        }
    }
    if let Some(actor) = actor {
        if !actor.trim().is_empty() {
            return actor.to_string();
        }
    }
    if let Ok(value) = std::env::var("BD_ACTOR") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    if let Ok(value) = std::env::var("BEADS_ACTOR") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    if let Some(name) = git_user_name() {
        return name;
    }
    if let Ok(value) = std::env::var("USER") {
        if !value.trim().is_empty() {
            return value;
        }
    }

    "unknown".to_string()
}

fn git_user_name() -> Option<String> {
    let output = Command::new("git")
        .args(["config", "--get", "user.name"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}
