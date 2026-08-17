use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;
use obr::cli::commands;
use obr::cli::{Cli, Commands, OutputFormat, command_requests_robot_json};
use obr::config;
use obr::logging::init_logging;
use obr::output::OutputContext;
use obr::sync::{
    auto_flush, auto_import_if_stale, auto_import_probe, auto_import_probe_refreshing_witnesses,
};
use obr::{BeadsError, Result, StructuredError};
use std::ffi::OsStr;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(not(windows))]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(not(test))]
const DISABLE_READ_ONLY_FAST_OPEN_ENV: &str = "OBR_DISABLE_READ_ONLY_FAST_OPEN";

#[allow(clippy::too_many_lines)]
fn main() {
    CompleteEnv::with_factory(Cli::command).complete();

    // Deprecation warnings fire from config discovery and env lookups, some of
    // them before `Cli::parse` returns, so the output mode is read straight
    // off the raw argument vector. Flags only — no value parsing.
    obr::legacy_compat::set_warning_output_mode_from_args(std::env::args());

    // Install SIGINT/SIGTERM/SIGHUP handlers before any storage opens so
    // an interrupt during a long-running command unwinds through main
    // and lets `SqliteStorage::Drop` flush the WAL (#270). The handler
    // is process-global and idempotent, so calling it from clap's
    // completion subprocess (above) would also be safe.
    obr::shutdown::install();

    let cli = Cli::parse();
    let json_error_mode = should_render_errors_as_json(&cli);
    let color_error_mode = should_color_human_errors_for_cli(&cli);
    let output_ctx = OutputContext::from_args(&cli);
    let is_mutating = is_mutating_command(&cli.command);
    let command_supports_auto_import = should_auto_import(&cli.command);

    // Initialize logging
    if let Err(e) = init_logging(cli.verbose, cli.quiet, None) {
        eprintln!("Failed to initialize logging: {e}");
    }
    // Text-mode commands are Unix filters: a reader that closes the pipe
    // early must end the process quietly, not as a SIGABRT core dump (#434).
    if should_restore_default_sigpipe(&cli, json_error_mode) {
        obr::shutdown::restore_default_sigpipe();
    }
    if let Commands::Sync(args) = &cli.command
        && let Err(error) = obr::cli::commands::sync::validate_sync_mode_args(args)
    {
        handle_error(&error, json_error_mode, color_error_mode);
    }

    let mut overrides = build_cli_overrides(&cli);

    // Phase 1: Startup & Discovery (One-time)
    let mut ctx = match StartupContext::init(&overrides) {
        Ok(ctx) => ctx,
        Err(e) => {
            if command_supports_auto_import {
                handle_error(&e, json_error_mode, color_error_mode);
            }
            StartupContext::empty(overrides.clone())
        }
    };

    let storage_enabled = ctx.is_initialized() && !ctx.no_db();
    let mut should_auto_import_now =
        command_supports_auto_import && !cli.allow_stale && !ctx.no_auto_import();
    let should_auto_flush_now = is_mutating && !ctx.no_auto_flush();
    let needs_preopened_storage_context = should_auto_import_now || should_auto_flush_now;
    let mut should_preopen_storage =
        should_preopen_storage(storage_enabled, needs_preopened_storage_context);
    // `obr serve` runs a long-lived MCP server that opens storage and performs
    // import/flush around each request itself. A preopened storage context
    // would own the database-family authority for the server's whole
    // lifetime, deadlocking the server's own same-process per-request
    // acquisitions and starving other workspace writers, so serve never
    // preopens (its pending-merge startup gate below still runs).
    #[cfg(feature = "mcp")]
    if matches!(cli.command, Commands::Serve(_)) {
        should_auto_import_now = false;
        should_preopen_storage = false;
    }
    let command_needs_write_lock = needs_write_lock(&cli.command);
    let no_db_jsonl_write = ctx.no_db() && no_db_jsonl_write_intent(&cli.command);

    // A committed merge is a durable multi-artifact saga. This pre-lock check
    // is advisory only: it can conservatively disable automatic sync early,
    // but the permitting decision is repeated below under inode-bound
    // database-family authority immediately before any storage open. The merge
    // command itself is the sole mutation allowed to resume that state; doctor
    // owns a richer dedicated finding/refusal surface.
    let pending_merge_disposition = pending_merge_startup_disposition(&cli.command);
    let mut pending_merge_warning_emitted = false;
    if ctx.is_initialized()
        && !ctx.no_db()
        && (command_needs_write_lock || should_preopen_storage)
        && pending_merge_disposition == PendingMergeStartupDisposition::ReadOnlyNoAutoSync
        && !matches!(cli.command, Commands::Doctor(_))
        && let Some(paths) = ctx.paths.as_ref()
    {
        match inspect_pending_sync_merge_for_startup(&paths.db_path) {
            Ok(Some(state)) => {
                emit_pending_sync_merge_warning(&state, json_error_mode);
                pending_merge_warning_emitted = true;
                // Read-only commands remain available, but their convenience
                // auto-import and every command-local storage fallback must
                // not advance either side of the saga. Force current-schema
                // read-only storage even when the user did not pass the
                // normal explicit stale/flush opt-outs.
                should_auto_import_now = false;
                force_pending_merge_read_only_mode(&mut overrides, &mut ctx);
                should_preopen_storage =
                    storage_enabled && supports_read_only_fast_open(&cli.command);
            }
            Ok(None) => {}
            Err(error) => {
                emit_pending_sync_merge_inspection_warning(&error, json_error_mode);
                should_auto_import_now = false;
                force_pending_merge_read_only_mode(&mut overrides, &mut ctx);
                should_preopen_storage =
                    storage_enabled && supports_read_only_fast_open(&cli.command);
            }
        }
    }

    // Phase 1.5: Acquire exclusive write lock before any DB-family open that
    // may apply schema, recover, quarantine sidecars, write metadata, or read
    // from SQLite while another process is in a write transaction.
    //
    // Issue #243 established the cross-process authority boundary: serialize
    // all mutating operations through a blocking flock on
    // `.beads/.write.lock`. Normal
    // storage open is not guaranteed read-only in recovery/schema paths, so
    // DB-family commands hold authority even when they first try the
    // current-schema read-only fast-open path. That keeps the pending-saga
    // verdict stable and prevents a fast-open miss from entering writable
    // recovery without a definitive gate.
    let ordinary_database_authority_required = should_acquire_startup_write_lock(
        command_needs_write_lock,
        should_preopen_storage,
        ctx.overrides.read_only_fast_open,
    );
    let pending_merge_mutation_gate_required =
        pending_merge_disposition == PendingMergeStartupDisposition::Refuse;
    let startup_database_authority_required = startup_database_authority_required(
        ctx.no_db(),
        ordinary_database_authority_required,
        no_db_jsonl_write,
        pending_merge_mutation_gate_required,
    );
    let write_lock = if startup_database_authority_required && ctx.is_initialized() {
        let lock_timeout = ctx.startup_write_lock_timeout(&cli.command);
        match ctx
            .obr_dir
            .as_deref()
            .zip(ctx.paths.as_ref())
            .map(|(obr_dir, paths)| {
                obr::sync::blocking_database_family_write_lock_with_timeout(
                    obr_dir,
                    &paths.db_path,
                    lock_timeout,
                )
                .map(Arc::new)
            }) {
            Some(Ok(lock)) => Some(lock),
            Some(Err(e)) => {
                // Round-3 fresh-eyes (`beads_rust-sexc`): when the
                // contended command is `obr doctor --repair`, surface the
                // structured `ConcurrencyLost` (exit code 5) documented
                // in `doctor_subsystems::exit_codes` instead of the
                // generic `BeadsError::Config` exit code. Other commands
                // still flow through `handle_error` unchanged.
                if let Commands::Doctor(doctor_args) = &cli.command {
                    let lock_path = ctx
                        .obr_dir
                        .as_ref()
                        .map(|d| d.join(".write.lock"))
                        .unwrap_or_else(|| PathBuf::from(".beads/.write.lock"));
                    let lock_display = lock_path.display().to_string();
                    if (doctor_args.repair || doctor_args.repair_indexes)
                        && !doctor_args.robot_triage
                    {
                        let command_name = if doctor_args.repair_indexes {
                            "--repair-indexes"
                        } else {
                            "--repair"
                        };
                        if json_error_mode {
                            let payload = serde_json::json!({
                                "ok": false,
                                "exit_code": obr::cli::commands::doctor_subsystems::exit_codes::DoctorExitCode::ConcurrencyLost.as_i32(),
                                "code": obr::cli::commands::doctor_subsystems::exit_codes::DoctorExitCode::ConcurrencyLost.as_str(),
                                "message": format!(
                                    "Refusing {command_name}: workspace write lock at {lock_display} is held by another process",
                                ),
                                "detail": e.to_string(),
                                "lock_path": lock_display,
                            });
                            // #336: structured JSON errors go to STDOUT in json
                            // mode so robot callers get one clean parseable stream.
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&payload)
                                    .unwrap_or_else(|_| payload.to_string())
                            );
                        } else {
                            eprintln!(
                                "Refusing {command_name}: workspace write lock at {lock_display} is held by another process. \
                                 Wait for the other obr invocation to finish or pass --lock-timeout to wait longer. \
                                 Underlying error: {e}",
                            );
                        }
                        obr::shutdown::exit_process(obr::cli::commands::doctor_subsystems::exit_codes::DoctorExitCode::ConcurrencyLost.as_i32());
                    }
                    if doctor_args.subcommand.is_none() {
                        if is_unwritable_write_lock_open_error(&lock_path, &e) {
                            emit_read_only_doctor_write_lock_diagnostic(
                                ctx.obr_dir.as_deref(),
                                &e,
                                json_error_mode,
                            );
                        }
                        if is_write_lock_contention_error(&lock_path, &e) {
                            emit_read_only_doctor_live_write_lock_diagnostic(
                                ctx.obr_dir.as_deref(),
                                &e,
                                json_error_mode,
                            );
                        }
                    }
                }
                handle_error(&e, json_error_mode, color_error_mode)
            }
            None => None,
        }
    } else {
        None
    };
    // Every non-merge DB-family caller inspects only after startup has obtained
    // its database-family write lock. This prevents a concurrent merge from
    // changing the receipt between the gate decision and storage open. Read
    // commands may continue on a classified pending receipt, but only in
    // current-schema read-only mode with automatic sync disabled. Inspection
    // uncertainty fails closed because a fast-open fallback could otherwise
    // recover or migrate the database before the command executes.
    if ctx.is_initialized()
        && ctx.no_db()
        && (no_db_jsonl_write || pending_merge_mutation_gate_required)
        && let Some(paths) = ctx.paths.as_ref()
    {
        let authority = write_lock.as_ref().unwrap_or_else(|| {
            handle_error(
                &BeadsError::SyncConflict {
                    message:
                        "Refusing no-DB mutation because no database-family authority is available for the pending sync-merge gate"
                            .to_string(),
                },
                json_error_mode,
                color_error_mode,
            )
        });
        match inspect_pending_sync_merge_for_startup_under_authority(&paths.db_path, authority) {
            Ok(Some(state)) => handle_error(
                &pending_sync_merge_no_db_refusal_error(&state),
                json_error_mode,
                color_error_mode,
            ),
            Ok(None) => {}
            Err(error @ BeadsError::SchemaMismatch { .. }) => {
                let routed = reviewed_schema_migration_required(error);
                handle_error(&routed, json_error_mode, color_error_mode)
            }
            Err(error) => handle_error(
                &BeadsError::SyncConflict {
                    message: format!(
                        "Refusing no-DB mutation because pending sync-merge state could not be inspected under database-family authority: {error}"
                    ),
                },
                json_error_mode,
                color_error_mode,
            ),
        }
    } else if ctx.is_initialized()
        && !ctx.no_db()
        && (command_needs_write_lock
            || should_preopen_storage
            || pending_merge_mutation_gate_required)
        && pending_merge_disposition != PendingMergeStartupDisposition::Resume
        && !matches!(cli.command, Commands::Doctor(_))
        // Read-only fast-open runs without the family authority; the
        // advisory path-only inspection above already classified pending
        // state for it, and a fast-open miss reacquires the authority (and
        // with it this definitive gate) before any writable fallback.
        && !(write_lock.is_none() && ctx.overrides.read_only_fast_open)
        && let Some(paths) = ctx.paths.as_ref()
    {
        let authority = write_lock.as_ref().unwrap_or_else(|| {
            handle_error(
                &BeadsError::SyncConflict {
                    message:
                        "Refusing storage open because no database-family authority is available for the live pending sync-merge gate"
                            .to_string(),
                },
                json_error_mode,
                color_error_mode,
            )
        });
        // This gate inspects by opening the database read-only, and fsqlite
        // refuses that while a hot journal sits beside it — so a file merely
        // NAMED `-journal` makes every command exit 6 with "rollback journal
        // has invalid magic", and no obr command can reach the workspace to fix
        // it. `quarantine_truncated_wal_sidecar` already handles the same
        // situation for a sub-header WAL, but it runs inside the storage open
        // below, which is one stage too late to help here.
        //
        // Only a journal whose magic PROVES it is not a rollback journal is
        // moved, and it is moved to `recovery/` rather than deleted. A real hot
        // journal, or any file this cannot judge, is left exactly where it is.
        // Both sidecars, for the same reason: each can block the read-only open
        // this gate performs, and the existing call to the WAL one lives inside
        // the storage open below — one stage too late to help here.
        crate::config::quarantine_truncated_wal_sidecar(&paths.db_path, &paths.obr_dir);
        crate::config::quarantine_bogus_journal_sidecar(&paths.db_path, &paths.obr_dir);
        match inspect_pending_sync_merge_for_startup_under_authority(&paths.db_path, authority) {
            Ok(Some(state))
                if pending_merge_disposition == PendingMergeStartupDisposition::Refuse =>
            {
                handle_error(
                    &pending_sync_merge_refusal_error(&state),
                    json_error_mode,
                    color_error_mode,
                )
            }
            Ok(Some(state)) => {
                if !pending_merge_warning_emitted {
                    emit_pending_sync_merge_warning(&state, json_error_mode);
                }
                should_auto_import_now = false;
                force_pending_merge_read_only_mode(&mut overrides, &mut ctx);
                should_preopen_storage =
                    storage_enabled && supports_read_only_fast_open(&cli.command);
            }
            Ok(None) => {}
            Err(error @ BeadsError::SchemaMismatch { .. }) => {
                let routed = reviewed_schema_migration_required(error);
                handle_error(&routed, json_error_mode, color_error_mode);
            }
            Err(error) => {
                handle_error(
                    &BeadsError::SyncConflict {
                        message: format!(
                            "Refusing storage open because pending sync-merge state could not be inspected under database-family authority: {error}"
                        ),
                    },
                    json_error_mode,
                    color_error_mode,
                );
            }
        }
    }
    if let Some(write_lock) = write_lock.as_ref()
        && let Some(obr_dir) = ctx.obr_dir.as_deref()
    {
        overrides.mark_database_family_lock_held(obr_dir, write_lock);
    }

    // `obr serve` only acquires startup authority for the pending-merge
    // mutation gate above. The server outlives startup, its bootstrap and
    // MCP mutation handlers take the same `.write.lock` flock through fresh
    // descriptors (a same-process conflict, not reentrant), and holding the
    // authority for the server's lifetime would also starve every other
    // workspace writer. The gate verdict is final here, so release both the
    // guard and the marked `Arc` clone before dispatch.
    #[cfg(feature = "mcp")]
    let write_lock = if matches!(cli.command, Commands::Serve(_)) {
        overrides.clear_database_family_lock_marker();
        ctx.overrides.clear_database_family_lock_marker();
        // Shadowing alone would keep the old binding — and its flock — alive
        // until `main` returns; move it out and release it now.
        drop(write_lock);
        None
    } else {
        write_lock
    };

    // Phase 2: Open Storage (One-time)
    let mut storage_result = if should_preopen_storage {
        match open_storage_from_ctx(&mut ctx, write_lock.as_ref()) {
            Ok(res) => Some(res),
            Err(e) => {
                if should_auto_import_now {
                    handle_error(&e, json_error_mode, color_error_mode);
                }
                None
            }
        }
    } else {
        None
    };

    // Phase 3: Auto-Import. Normal staleness probes can opportunistically
    // refresh JSONL witness metadata. Read-only startup probes skip that
    // refresh and reopen writable storage only when an import is actually
    // needed.
    if let Some(paths) = ctx.paths.as_ref()
        && should_auto_import_now
        && storage_result.is_some()
    {
        let allow_external_jsonl = config::implicit_external_jsonl_allowed(
            &paths.obr_dir,
            &paths.db_path,
            &paths.jsonl_path,
        );
        let mut auto_import_write_lock = storage_result
            .as_ref()
            .and_then(config::OpenStorageResult::retained_database_write_authority);
        if !ctx.overrides.read_only_fast_open
            && write_lock.is_none()
            && auto_import_write_lock.is_none()
        {
            let lock_timeout = ctx.write_lock_timeout();
            auto_import_write_lock = match ctx.obr_dir.as_deref().map(|obr_dir| {
                obr::sync::blocking_database_family_write_lock_with_timeout(
                    obr_dir,
                    &paths.db_path,
                    lock_timeout,
                )
                .map(Arc::new)
            }) {
                Some(Ok(lock)) => Some(lock),
                Some(Err(e)) => handle_error(&e, json_error_mode, color_error_mode),
                None => None,
            };
        }
        let mut should_attempt_auto_import = {
            match storage_result.as_mut() {
                Some(res) if ctx.overrides.read_only_fast_open => auto_import_probe(
                    &res.storage,
                    &paths.obr_dir,
                    &paths.jsonl_path,
                    allow_external_jsonl,
                )
                .unwrap_or(true),
                Some(res) => auto_import_probe_refreshing_witnesses(
                    &mut res.storage,
                    &paths.obr_dir,
                    &paths.jsonl_path,
                    allow_external_jsonl,
                )
                .unwrap_or(true),
                None => false,
            }
        };

        if should_attempt_auto_import {
            if ctx.overrides.read_only_fast_open
                && write_lock.is_none()
                && auto_import_write_lock.is_none()
            {
                let lock_timeout = ctx.write_lock_timeout();
                auto_import_write_lock = match ctx.obr_dir.as_deref().map(|obr_dir| {
                    obr::sync::blocking_database_family_write_lock_with_timeout(
                        obr_dir,
                        &paths.db_path,
                        lock_timeout,
                    )
                    .map(Arc::new)
                }) {
                    Some(Ok(lock)) => Some(lock),
                    Some(Err(e)) => handle_error(&e, json_error_mode, color_error_mode),
                    None => None,
                };
            }

            // The first read-only probe is deliberately advisory: JSONL, the
            // pending-merge receipt, or even the canonical database inode can
            // change before we join the writer queue. Reopen the canonical
            // database under the acquired family authority, then reclassify
            // both freshness and pending state on that protected handle. A
            // classified pending receipt keeps the command read-only; probe
            // uncertainty is an error because serving the database anyway
            // could return stale state.
            if ctx.overrides.read_only_fast_open
                && let Some(authority) = auto_import_write_lock.as_ref()
            {
                match reopen_and_reprobe_fast_open_auto_import_under_authority(
                    &mut storage_result,
                    paths,
                    &ctx.overrides,
                    authority,
                    allow_external_jsonl,
                ) {
                    Ok(reprobe) => {
                        should_attempt_auto_import = apply_fast_open_auto_import_reprobe(
                            reprobe,
                            &mut pending_merge_warning_emitted,
                            &mut overrides,
                            &mut ctx.overrides,
                            json_error_mode,
                        );
                    }
                    Err(error) => handle_error(&error, json_error_mode, color_error_mode),
                }
            }
        }

        if should_attempt_auto_import {
            if ctx.overrides.read_only_fast_open {
                let mut writable_overrides = ctx.overrides.clone();
                writable_overrides.read_only_fast_open = false;
                let authority = auto_import_write_lock
                    .as_ref()
                    .or(write_lock.as_ref())
                    .unwrap_or_else(|| {
                        handle_error(
                            &BeadsError::SyncConflict {
                                message:
                                    "Writable fast-open reopen has no database-family authority"
                                        .to_string(),
                            },
                            json_error_mode,
                            color_error_mode,
                        )
                    });
                writable_overrides.mark_database_family_lock_held(&paths.obr_dir, authority);
                let frozen_startup = storage_result
                    .as_ref()
                    .map(config::OpenStorageResult::retained_startup_config)
                    .unwrap_or_else(|| {
                        handle_error(
                            &BeadsError::SyncConflict {
                                message: "Writable fast-open reopen lost its startup snapshot"
                                    .to_string(),
                            },
                            json_error_mode,
                            color_error_mode,
                        )
                    });
                drop(storage_result.take());
                match config::open_storage_with_startup_config_under_write_lock(
                    frozen_startup,
                    &writable_overrides,
                    false,
                    authority,
                ) {
                    Ok(writable_res) => storage_result = Some(writable_res),
                    Err(e) => handle_error(&e, json_error_mode, color_error_mode),
                }
            }

            let _ = auto_import_write_lock.as_ref();
            let sync_lock = match ctx.obr_dir.as_deref().map(obr::sync::try_sync_lock) {
                Some(Ok(Some(lock))) => Some(lock),
                Some(Ok(None)) => {
                    tracing::debug!("Auto-import skipped because .sync.lock is held");
                    None
                }
                Some(Err(e)) => handle_error(&e, json_error_mode, color_error_mode),
                None => None,
            };
            if sync_lock.is_some()
                && let Some(res) = storage_result.as_mut()
            {
                let expected_prefix = match resolve_auto_import_expected_prefix(res, &ctx.overrides)
                {
                    Ok(prefix) => Some(prefix),
                    Err(e) => {
                        handle_error(&e, json_error_mode, color_error_mode);
                    }
                };
                let outcome = auto_import_if_stale(
                    &mut res.storage,
                    &paths.obr_dir,
                    &paths.jsonl_path,
                    expected_prefix.as_deref(),
                    allow_external_jsonl,
                    false,
                    false,
                );
                if let Err(e) = outcome {
                    handle_error(&e, json_error_mode, color_error_mode);
                }
            }
            // sync_lock drops here, releasing the advisory lock before command execution
        }
    }

    // Phase 4: Command Execution
    let result = match cli.command {
        Commands::Init {
            prefix,
            force,
            backend: _,
        } => commands::init::execute(prefix, force, None, &output_ctx),
        Commands::Create(args) => {
            execute_create_command(&args, &overrides, &output_ctx, &mut storage_result)
        }
        Commands::Update(args) => commands::update::execute(&args, &overrides, &output_ctx),
        Commands::Delete(args) => {
            commands::delete::execute(&args, cli.json, &overrides, &output_ctx)
        }
        Commands::List(args) => {
            if let Some(res) = storage_result.as_ref() {
                commands::list::execute_with_storage(&args, &overrides, &output_ctx, res)
            } else {
                commands::list::execute(&args, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Comments(args) => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                match commands::comments::execute_with_storage_ctx(
                    &args,
                    cli.json,
                    &overrides,
                    &output_ctx,
                    obr_dir,
                    res,
                ) {
                    Ok(true) => Ok(()),
                    Ok(false) => {
                        commands::comments::execute(&args, cli.json, &overrides, &output_ctx)
                    }
                    Err(err) => Err(err),
                }
            } else {
                commands::comments::execute(&args, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Search(args) => {
            if let Some(res) = storage_result.as_ref() {
                commands::search::execute_with_storage_ctx(&args, &overrides, &output_ctx, res)
            } else {
                commands::search::execute(&args, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Show(args) => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                commands::show::execute_with_storage_ctx(
                    &args,
                    &overrides,
                    &output_ctx,
                    obr_dir,
                    res,
                )
            } else {
                commands::show::execute(&args, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Close(args) => {
            commands::close::execute_cli(&args, cli.json || args.robot, &overrides, &output_ctx)
        }
        Commands::Reopen(args) => {
            commands::reopen::execute(&args, cli.json || args.robot, &overrides, &output_ctx)
        }
        Commands::Q(args) => commands::q::execute(args, &overrides, &output_ctx),
        Commands::Dep { command } => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                match commands::dep::execute_with_storage_ctx(
                    &command,
                    cli.json,
                    &overrides,
                    &output_ctx,
                    obr_dir,
                    res,
                ) {
                    Ok(true) => Ok(()),
                    Ok(false) => {
                        commands::dep::execute(&command, cli.json, &overrides, &output_ctx)
                    }
                    Err(err) => Err(err),
                }
            } else {
                commands::dep::execute(&command, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Epic { command } => {
            if let Some(res) = storage_result.as_ref() {
                match commands::epic::execute_with_storage_ctx(
                    &command,
                    &overrides,
                    &output_ctx,
                    res,
                ) {
                    Ok(true) => Ok(()),
                    Ok(false) => {
                        commands::epic::execute(&command, cli.json, &overrides, &output_ctx)
                    }
                    Err(err) => Err(err),
                }
            } else {
                commands::epic::execute(&command, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Gate { command } => commands::gate::execute(&command, &overrides, &output_ctx),
        Commands::Capacity { command } => {
            commands::capacity::execute(&command, &overrides, &output_ctx)
        }
        Commands::Label { command } => {
            if let Some(res) = storage_result.as_ref() {
                match commands::label::execute_with_storage(
                    &command,
                    cli.json,
                    &output_ctx,
                    &res.storage,
                ) {
                    Ok(true) => Ok(()),
                    Ok(false) => {
                        commands::label::execute(&command, cli.json, &overrides, &output_ctx)
                    }
                    Err(err) => Err(err),
                }
            } else {
                commands::label::execute(&command, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Coordination { command } => match command {
            obr::cli::CoordinationCommands::Status(args) => {
                if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref())
                {
                    commands::coordination::execute_status_with_storage_ctx(
                        &args,
                        &overrides,
                        &output_ctx,
                        obr_dir,
                        res,
                    )
                } else {
                    commands::coordination::execute_status(&args, &overrides, &output_ctx)
                }
            }
        },
        Commands::Count(args) => {
            if let Some(res) = storage_result.as_ref() {
                commands::count::execute_with_storage(&args, &output_ctx, &res.storage)
            } else {
                commands::count::execute(&args, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Capabilities(args) => commands::capabilities::execute(&args, &output_ctx),
        Commands::Stale(args) => storage_result.as_ref().map_or_else(
            || commands::stale::execute(&args, &overrides, &output_ctx),
            |res| commands::stale::execute_with_storage(&args, &output_ctx, &res.storage),
        ),
        Commands::Lint(args) => {
            if let Some(res) = storage_result.as_ref() {
                match commands::lint::execute_with_storage_ctx(&args, &output_ctx, res) {
                    Ok(true) => Ok(()),
                    Ok(false) => commands::lint::execute(&args, cli.json, &overrides, &output_ctx),
                    Err(err) => Err(err),
                }
            } else {
                commands::lint::execute(&args, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Ready(args) => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                commands::ready::execute_with_storage_ctx(
                    &args,
                    &overrides,
                    &output_ctx,
                    obr_dir,
                    res,
                )
            } else {
                commands::ready::execute(&args, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::RobotDocs { command } => commands::robot_docs::execute(&command, &output_ctx),
        Commands::Scheduler(args) => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                commands::scheduler::execute_with_storage_ctx(
                    &args,
                    &overrides,
                    &output_ctx,
                    obr_dir,
                    res,
                )
            } else {
                commands::scheduler::execute(&args, cli.json, &overrides, &output_ctx)
            }
        }
        Commands::Blocked(args) => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                commands::blocked::execute_with_storage_ctx(
                    &args,
                    &overrides,
                    &output_ctx,
                    obr_dir,
                    res,
                )
            } else {
                commands::blocked::execute(&args, cli.json || args.robot, &overrides, &output_ctx)
            }
        }
        Commands::Sync(args) => commands::sync::execute(
            &args,
            cli.json,
            &overrides,
            &output_ctx,
            write_lock.is_some(),
        ),
        Commands::VcsStatus(args) => commands::vcs::execute(&args, &overrides, &output_ctx),
        Commands::Doctor(args) => commands::doctor::execute(&args, &overrides, &output_ctx),
        Commands::Info(args) => commands::info::execute(&args, &overrides, &output_ctx),
        Commands::Schema(args) => commands::schema::execute(&args, &overrides, &output_ctx),
        Commands::Where => commands::r#where::execute(&overrides, &output_ctx),
        Commands::Version(args) => commands::version::execute(&args, &output_ctx),

        #[cfg(feature = "mcp")]
        Commands::Serve(args) => obr::mcp::run_serve(&args, &overrides),

        Commands::Completions(args) => commands::completions::execute(&args, &output_ctx),
        Commands::Audit { command } => {
            commands::audit::execute(&command, cli.json, &overrides, &output_ctx)
        }
        Commands::Stats(args) | Commands::Status(args) => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                commands::stats::execute_with_storage_ctx(
                    &args,
                    &overrides,
                    &output_ctx,
                    obr_dir,
                    res,
                )
            } else {
                commands::stats::execute(&args, cli.json || args.robot, &overrides, &output_ctx)
            }
        }
        Commands::Config { command } => {
            commands::config::execute(&command, cli.json, &overrides, &output_ctx)
        }
        Commands::History(args) => commands::history::execute(args, &overrides, &output_ctx),
        Commands::Defer(args) => {
            commands::defer::execute_defer(&args, cli.json || args.robot, &overrides, &output_ctx)
        }
        Commands::Undefer(args) => {
            commands::defer::execute_undefer(&args, cli.json || args.robot, &overrides, &output_ctx)
        }
        Commands::Orphans(args) if !args.fix => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                commands::orphans::execute_with_storage_ctx(
                    &args,
                    cli.json || args.robot,
                    &overrides,
                    &output_ctx,
                    obr_dir,
                    res,
                )
            } else {
                commands::orphans::execute(&args, cli.json || args.robot, &overrides, &output_ctx)
            }
        }
        Commands::Orphans(args) => {
            commands::orphans::execute(&args, cli.json || args.robot, &overrides, &output_ctx)
        }
        Commands::Changelog(args) => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                commands::changelog::execute_with_storage_ctx(
                    &args,
                    cli.json || args.robot,
                    &output_ctx,
                    obr_dir,
                    res,
                )
            } else {
                commands::changelog::execute(&args, cli.json || args.robot, &overrides, &output_ctx)
            }
        }
        Commands::Query { command } => {
            if is_read_only_query_command(&command) {
                if let Some(res) = storage_result.as_mut() {
                    commands::query::execute_with_storage_ctx(
                        &command,
                        &overrides,
                        &output_ctx,
                        res,
                    )
                } else {
                    commands::query::execute(&command, &overrides, &output_ctx)
                }
            } else {
                commands::query::execute(&command, &overrides, &output_ctx)
            }
        }
        Commands::Graph(args) => {
            if let (Some(res), Some(obr_dir)) = (storage_result.as_ref(), ctx.obr_dir.as_ref()) {
                commands::graph::execute_with_storage_ctx(
                    &args,
                    &overrides,
                    &output_ctx,
                    obr_dir,
                    res,
                )
            } else {
                commands::graph::execute(&args, &overrides, &output_ctx)
            }
        }
        Commands::Agents(args) => {
            let agents_args = commands::agents::AgentsArgs {
                add: args.add,
                remove: args.remove,
                update: args.update,
                check: args.check,
                dry_run: args.dry_run,
                force: args.force,
            };
            commands::agents::execute(&agents_args, &output_ctx)
        }
    };

    // Handle command result
    if let Err(e) = result {
        handle_error(&e, json_error_mode, color_error_mode);
    }

    // Cooperative shutdown: if a SIGINT/SIGTERM/SIGHUP arrived while
    // the command was executing, skip the auto-flush phase and let
    // every local — including `storage_result` — drop on the way out
    // of `main`, so `SqliteStorage::Drop` checkpoints the WAL before
    // the process exits (#270).
    if let Some(exit_code) = obr::shutdown::exit_code() {
        drop(storage_result);
        drop(write_lock);
        obr::shutdown::exit_process(exit_code);
    }

    // Phase 5: Auto-Flush (with advisory flock to serialize concurrent access)
    if is_mutating
        && !ctx.no_auto_flush()
        && let (Some(res), Some(paths)) = (storage_result.as_mut(), ctx.paths.as_ref())
    {
        let sync_lock = match obr::sync::try_sync_lock(&paths.obr_dir) {
            Ok(Some(lock)) => Some(lock),
            Ok(None) => {
                let err = BeadsError::Config(format!(
                    "Automatic JSONL export skipped because sync lock at {} is held by another process",
                    paths.obr_dir.join(".sync.lock").display()
                ));
                commands::report_auto_flush_failure(
                    &output_ctx,
                    &paths.obr_dir,
                    &paths.jsonl_path,
                    &err,
                );
                None
            }
            Err(e) => {
                commands::report_auto_flush_failure(
                    &output_ctx,
                    &paths.obr_dir,
                    &paths.jsonl_path,
                    &e,
                );
                None
            }
        };

        if let Some(_sync_lock) = sync_lock
            && let Err(e) = auto_flush(
                &mut res.storage,
                &paths.obr_dir,
                &paths.jsonl_path,
                config::implicit_external_jsonl_allowed(
                    &paths.obr_dir,
                    &paths.db_path,
                    &paths.jsonl_path,
                ),
            )
        {
            commands::report_auto_flush_failure(&output_ctx, &paths.obr_dir, &paths.jsonl_path, &e);
        }
    }

    if let Some(err) = obr::output::take_output_serialization_failure() {
        obr::shutdown::exit_process(err.exit_code());
    }

    // A command emitted its normal output and any auto-flush has now completed,
    // but detected a condition that must surface to scripted callers via a
    // non-zero exit code (e.g. `dep cycles` with cycles present, or `create -f`
    // that dropped declared dependency edges) — see #368. Drop storage first so
    // `SqliteStorage::Drop` checkpoints the WAL before the process exits (#270).
    if let Some(exit_code) = obr::output::take_pending_exit_code() {
        drop(storage_result);
        drop(write_lock);
        obr::shutdown::exit_process(exit_code);
    }

    // Successful exit goes through the same funnel as every other exit
    // path (#439): on Windows, letting `main` return would reach the CRT
    // `exit()` teardown, where an atexit/TLS destructor joining a thread
    // that `ExitProcess` already terminated aborts with 0xC0000409 and
    // corrupts the exit code of a command that worked. Storage is dropped
    // first so `SqliteStorage::Drop` checkpoints the WAL (#270).
    drop(storage_result);
    drop(write_lock);
    obr::shutdown::exit_process(0);
}

struct StartupContext {
    overrides: config::CliOverrides,
    startup: Option<config::StartupConfig>,
    obr_dir: Option<PathBuf>,
    paths: Option<config::ConfigPaths>,
    config: Option<config::ConfigLayer>,
}

impl StartupContext {
    fn init(overrides: &config::CliOverrides) -> Result<Self> {
        let obr_dir = config::discover_obr_dir_with_cli(overrides)?;
        let startup = config::load_startup_config_with_paths(&obr_dir, overrides.db.as_ref())?;

        // Merge startup config with CLI overrides to form the effective bootstrap config
        let mut final_config = startup.merged_config.clone();
        final_config.merge_from(&overrides.as_layer());
        let paths = startup.paths.clone();

        Ok(Self {
            overrides: overrides.clone(),
            startup: Some(startup),
            obr_dir: Some(obr_dir),
            paths: Some(paths),
            config: Some(final_config),
        })
    }

    fn empty(overrides: config::CliOverrides) -> Self {
        Self {
            overrides,
            startup: None,
            obr_dir: None,
            paths: None,
            config: None,
        }
    }

    fn is_initialized(&self) -> bool {
        self.obr_dir.is_some()
    }

    fn no_db(&self) -> bool {
        self.config
            .as_ref()
            .and_then(config::no_db_from_layer)
            .unwrap_or(false)
    }

    fn no_auto_import(&self) -> bool {
        self.config
            .as_ref()
            .and_then(config::no_auto_import_from_layer)
            .unwrap_or(false)
    }

    fn no_auto_flush(&self) -> bool {
        self.config
            .as_ref()
            .and_then(config::no_auto_flush_from_layer)
            .unwrap_or(false)
    }

    fn configured_write_lock_timeout(&self) -> Option<u64> {
        self.config
            .as_ref()
            .and_then(config::lock_timeout_from_layer)
            .or(self.overrides.lock_timeout)
    }

    fn write_lock_timeout(&self) -> Option<u64> {
        self.configured_write_lock_timeout()
            .or(Some(obr::sync::default_write_lock_timeout_ms()))
    }

    fn startup_write_lock_timeout(&self, command: &Commands) -> Option<u64> {
        if command_is_doctor_repair(command) {
            self.configured_write_lock_timeout().or(Some(0))
        } else {
            self.write_lock_timeout()
        }
    }
}

fn command_is_doctor_repair(command: &Commands) -> bool {
    matches!(command, Commands::Doctor(args) if (args.repair || args.repair_indexes) && !args.robot_triage)
}

const fn doctor_subcommand_needs_write_lock(args: &obr::cli::DoctorArgs) -> bool {
    match &args.subcommand {
        // `--repair` / `--fix` and `--repair-indexes` mutate and need authority.
        // A bare `obr doctor` does not, and claiming the lock for it was an
        // accident of shape rather than a decision: repair is a FLAG on the
        // `None` subcommand, not a subcommand of its own, so this arm could not
        // see the difference and conservatively took the lock for both.
        //
        // It is not free conservatism. `write_lock` exists to report an orphaned
        // `.write.lock`, and it probes by trying to take it — so holding the lock
        // here made the check collide with its own caller and answer "held by a
        // live process" for every workspace, in exactly the case it exists to
        // detect. A read-only diagnostic that cannot be run against a workspace
        // another process is using is also the wrong tool for the multi-agent
        // repositories this is aimed at.
        //
        // Safe because nothing here depends on holding it: `is_mutating_command`
        // and `should_auto_import` are both false for Doctor, so
        // `should_preopen_storage` is already false and no storage is opened
        // under this lock. If a read-only open later turns out to need writing,
        // the auto-import escalation path acquires the lock then and reopens
        // writably — the Phase 1.5 invariant is preserved by escalation, not by
        // taking the lock up front.
        None => args.repair || args.repair_indexes,
        Some(
            obr::cli::DoctorSubcommand::Undo(_) | obr::cli::DoctorSubcommand::MigrateSchema(_),
        ) => true,
        Some(
            obr::cli::DoctorSubcommand::Capabilities(_)
            | obr::cli::DoctorSubcommand::RobotDocs(_)
            | obr::cli::DoctorSubcommand::Health(_)
            | obr::cli::DoctorSubcommand::Ls(_)
            | obr::cli::DoctorSubcommand::Explain(_),
        ) => false,
    }
}

fn open_storage_from_ctx(
    ctx: &mut StartupContext,
    write_authority: Option<&Arc<obr::sync::DatabaseFamilyWriteLock>>,
) -> Result<config::OpenStorageResult> {
    let startup = ctx.startup.take().ok_or(BeadsError::NotInitialized)?;
    if let Some(write_authority) = write_authority {
        config::open_storage_with_startup_config_under_write_lock(
            startup,
            &ctx.overrides,
            false,
            write_authority,
        )
    } else {
        config::open_storage_with_startup_config(startup, &ctx.overrides, false)
    }
}

fn resolve_auto_import_expected_prefix(
    storage_result: &config::OpenStorageResult,
    cli: &config::CliOverrides,
) -> Result<String> {
    let layer = storage_result.load_config(cli)?;
    Ok(config::id_config_from_layer(&layer).prefix)
}

fn execute_create_command(
    args: &obr::cli::CreateArgs,
    overrides: &config::CliOverrides,
    output_ctx: &OutputContext,
    storage_result: &mut Option<config::OpenStorageResult>,
) -> Result<()> {
    commands::create::execute_with_storage(args, overrides, output_ctx, storage_result.take())
}

const fn should_preopen_storage(
    storage_enabled: bool,
    needs_preopened_storage_context: bool,
) -> bool {
    storage_enabled && needs_preopened_storage_context
}

const fn sync_mode_opens_storage(args: &obr::cli::SyncArgs) -> bool {
    args.flush_only || args.import_only || args.merge || args.reconcile || args.status
}

const fn should_acquire_startup_write_lock(
    command_needs_write_lock: bool,
    should_preopen_storage: bool,
    read_only_fast_open: bool,
) -> bool {
    // Read-only fast-open commands try the current-schema read-only engine
    // path before joining the writer-lock queue (1b75961a, reverted by the
    // 251b501b rescue merge and restored here). A fast-open miss clears
    // `read_only_fast_open` and reacquires the family authority inside
    // config before any recovery or writable fallback, and pending-merge
    // mutation gates still force the lock through
    // `startup_database_authority_required`.
    !read_only_fast_open && (command_needs_write_lock || should_preopen_storage)
}

// The startup gate genuinely composes four independent boolean facts; a
// struct would only rename them without adding meaning at the single call
// site.
#[allow(clippy::fn_params_excessive_bools)]
const fn startup_database_authority_required(
    no_db: bool,
    ordinary_database_authority_required: bool,
    no_db_jsonl_write: bool,
    pending_merge_mutation_gate_required: bool,
) -> bool {
    if pending_merge_mutation_gate_required {
        return true;
    }
    if no_db {
        no_db_jsonl_write
    } else {
        ordinary_database_authority_required
    }
}

/// Determine if a command potentially mutates data and triggers auto-flush.
const fn is_mutating_command(cmd: &Commands) -> bool {
    match cmd {
        Commands::Create(_)
        | Commands::Update(_)
        | Commands::Delete(_)
        | Commands::Close(_)
        | Commands::Reopen(_)
        | Commands::Q(_)
        | Commands::Defer(_)
        | Commands::Undefer(_) => true,
        Commands::Dep { command } => matches!(
            command,
            obr::cli::DepCommands::Add(_)
                | obr::cli::DepCommands::Import(_)
                | obr::cli::DepCommands::Remove(_)
        ),
        Commands::Label { command } => matches!(
            command,
            obr::cli::LabelCommands::Add(_)
                | obr::cli::LabelCommands::Remove(_)
                | obr::cli::LabelCommands::Rename(_)
        ),
        Commands::Comments(args) => matches!(
            args.command.as_ref(),
            Some(obr::cli::CommentCommands::Add(_))
        ),
        Commands::Epic { command } => matches!(
            command,
            obr::cli::EpicCommands::CloseEligible(args) if !args.dry_run
        ),
        Commands::Orphans(args) => args.fix,
        _ => false,
    }
}

const fn command_is_sync_merge(cmd: &Commands) -> bool {
    matches!(cmd, Commands::Sync(args) if args.merge)
}

/// Return true for every command that can mutate tracker or workspace state
/// while a committed sync merge is awaiting artifact reconciliation.
///
/// This is deliberately broader than [`is_mutating_command`], which only
/// controls the ordinary post-command auto-flush. Explicit sync modes, saved
/// queries, gate reports, doctor recovery, history/config maintenance, and
/// append-only audit commands mutate without using that auto-flush path.
const fn command_must_refuse_during_pending_merge(cmd: &Commands) -> bool {
    if is_mutating_command(cmd) {
        return true;
    }
    match cmd {
        Commands::Init { .. } => true,
        Commands::Sync(args) => {
            args.flush_only
                || args.import_only
                || (args.reconcile && !args.dry_run)
                || (args.reconcile_additive && args.apply)
        }
        Commands::Doctor(args) => {
            ((!args.robot_triage && (args.repair || args.repair_indexes)) && !args.dry_run)
                || matches!(
                    args.subcommand.as_ref(),
                    Some(obr::cli::DoctorSubcommand::Undo(undo)) if !undo.dry_run
                )
                || matches!(
                    args.subcommand.as_ref(),
                    Some(obr::cli::DoctorSubcommand::MigrateSchema(_))
                )
        }
        Commands::Gate { command } => {
            matches!(command, obr::cli::GateCommands::Report(_))
        }
        Commands::Capacity { command } => is_mutating_capacity_command(command),
        Commands::Query { command } => matches!(
            command,
            obr::cli::QueryCommands::Save(_) | obr::cli::QueryCommands::Delete(_)
        ),
        Commands::Config { command } => matches!(
            command,
            obr::cli::ConfigCommands::Set { .. }
                | obr::cli::ConfigCommands::Delete { .. }
                | obr::cli::ConfigCommands::Edit
        ),
        Commands::History(args) => matches!(
            args.command,
            Some(
                obr::cli::HistoryCommands::Restore { .. } | obr::cli::HistoryCommands::Prune { .. }
            )
        ),
        Commands::Audit { command } => matches!(
            command,
            obr::cli::AuditCommands::Record(_)
                | obr::cli::AuditCommands::Coordination(_)
                | obr::cli::AuditCommands::Label(_)
        ),
        Commands::Agents(args) => !args.dry_run && (args.add || args.remove || args.update),
        #[cfg(feature = "mcp")]
        Commands::Serve(_) => true,
        _ => false,
    }
}

/// Keep capacity mutation classification exhaustive so a future subcommand
/// cannot silently bypass the pending-merge refusal gate.
const fn is_mutating_capacity_command(command: &obr::cli::CapacityCommands) -> bool {
    match command {
        obr::cli::CapacityCommands::Exempt(_)
        | obr::cli::CapacityCommands::Renew(_)
        | obr::cli::CapacityCommands::Revoke(_) => true,
        obr::cli::CapacityCommands::Exemptions(_) => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingMergeStartupDisposition {
    /// The dedicated merge command owns receipt validation and saga resume.
    Resume,
    /// A tracker/workspace mutation must fail before storage open.
    Refuse,
    /// A read-only command may run, but automatic import/export is disabled.
    ReadOnlyNoAutoSync,
}

const fn pending_merge_startup_disposition(cmd: &Commands) -> PendingMergeStartupDisposition {
    if command_is_sync_merge(cmd) {
        PendingMergeStartupDisposition::Resume
    } else if command_must_refuse_during_pending_merge(cmd) {
        PendingMergeStartupDisposition::Refuse
    } else {
        PendingMergeStartupDisposition::ReadOnlyNoAutoSync
    }
}

fn force_pending_merge_read_only_mode(
    overrides: &mut config::CliOverrides,
    ctx: &mut StartupContext,
) {
    force_pending_merge_read_only_overrides(overrides);
    force_pending_merge_read_only_overrides(&mut ctx.overrides);
}

fn force_pending_merge_read_only_overrides(overrides: &mut config::CliOverrides) {
    overrides.no_auto_import = Some(true);
    overrides.no_auto_flush = Some(true);
    overrides.read_only_fast_open = true;
}

fn pending_sync_merge_refusal_error(state: &commands::doctor::PendingSyncMergeState) -> BeadsError {
    let phase = state.phase.as_deref().unwrap_or("unknown");
    let receipt = state.receipt_id.as_deref().unwrap_or("unvalidated");
    BeadsError::SyncConflict {
        message: format!(
            "Refusing non-merge mutation while pending sync-merge state is {} (phase={phase}, receipt={receipt}): {}. Run `obr sync --merge` to resume and verify artifact reconciliation first",
            state.condition_name(),
            state.diagnostic
        ),
    }
}

fn reviewed_schema_migration_required(source: BeadsError) -> BeadsError {
    source.reviewed_schema_migration_required()
}

fn pending_sync_merge_no_db_refusal_error(
    state: &commands::doctor::PendingSyncMergeState,
) -> BeadsError {
    BeadsError::SyncConflict {
        message: format!(
            "Refusing no-DB JSONL mutation while pending sync-merge state is {}: {}. The database saga owns this JSONL generation; rerun `obr sync --merge` without `--no-db` to reconcile it first",
            state.condition_name(),
            state.diagnostic
        ),
    }
}

fn inspect_pending_sync_merge_for_startup(
    db_path: &Path,
) -> Result<Option<commands::doctor::PendingSyncMergeState>> {
    // A missing database is the normal pre-import state for a JSONL-only
    // checkout and cannot contain a pending merge receipt, so the advisory
    // startup inspection must not push such workspaces into degraded
    // read-only mode (#414, #409 cluster A). Doctor still reports the
    // missing database as a finding through its stricter public inspector.
    match fs::symlink_metadata(db_path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        _ => commands::doctor::inspect_pending_sync_merge_at_path(db_path),
    }
}

fn inspect_pending_sync_merge_for_startup_under_authority(
    db_path: &Path,
    authority: &Arc<obr::sync::DatabaseFamilyWriteLock>,
) -> Result<Option<commands::doctor::PendingSyncMergeState>> {
    // Binding an exactly absent database under the held family authority
    // makes that absence definitive for the startup gate. The subsequent
    // writable open may then initialize/import it without leaving an
    // unchecked replacement window.
    if authority.bind_database_inode_for_mutation()? {
        authority.verify_database_authority()?;
        // A missing main database only proves "no pending merge" when no
        // sidecar outlives it. An orphaned WAL can still carry the committed
        // receipt, and this gate guards a no-DB JSONL write that would then
        // publish pre-merge bytes over merged truth.
        if authority.database_family_is_absent()? {
            return Ok(None);
        }
        return Err(BeadsError::SyncConflict {
            message:
                "Pending sync-merge state is unknown because the database is missing while its sidecars still hold data"
                    .to_string(),
        });
    }
    commands::doctor::inspect_pending_sync_merge_under_authority(db_path, authority)
}

#[derive(Debug)]
enum FastOpenAutoImportReprobe {
    Current,
    ImportRequired,
    Pending(commands::doctor::PendingSyncMergeState),
}

fn reopen_and_reprobe_fast_open_auto_import_under_authority(
    storage_result: &mut Option<config::OpenStorageResult>,
    paths: &config::ConfigPaths,
    overrides: &config::CliOverrides,
    authority: &Arc<obr::sync::DatabaseFamilyWriteLock>,
    allow_external_jsonl: bool,
) -> Result<FastOpenAutoImportReprobe> {
    let mut canonical_overrides = overrides.clone();
    canonical_overrides.read_only_fast_open = true;
    canonical_overrides.mark_database_family_lock_held(&paths.obr_dir, authority);

    // The pre-lock read-only connection may refer to an inode that a writer
    // replaced while this process waited for authority. Never make a
    // post-lock freshness decision through that orphaned handle. Retain the
    // original startup snapshot so config drift cannot mix DB and JSONL
    // routing generations during the protected reopen.
    let frozen_startup = storage_result
        .as_ref()
        .map(config::OpenStorageResult::retained_startup_config)
        .ok_or_else(|| BeadsError::SyncConflict {
            message: "Canonical fast-open storage disappeared before its protected reopen"
                .to_string(),
        })?;
    drop(storage_result.take());
    let reopened = config::open_storage_with_startup_config_under_write_lock(
        frozen_startup,
        &canonical_overrides,
        false,
        authority,
    )?;
    *storage_result = Some(reopened);

    if let Some(state) =
        inspect_pending_sync_merge_for_startup_under_authority(&paths.db_path, authority)?
    {
        return Ok(FastOpenAutoImportReprobe::Pending(state));
    }

    let Some(storage_result) = storage_result.as_ref() else {
        return Err(BeadsError::SyncConflict {
            message: "Canonical fast-open storage disappeared before the protected freshness probe"
                .to_string(),
        });
    };
    if auto_import_probe(
        &storage_result.storage,
        &paths.obr_dir,
        &paths.jsonl_path,
        allow_external_jsonl,
    )? {
        Ok(FastOpenAutoImportReprobe::ImportRequired)
    } else {
        Ok(FastOpenAutoImportReprobe::Current)
    }
}

fn apply_fast_open_auto_import_reprobe(
    reprobe: FastOpenAutoImportReprobe,
    pending_merge_warning_emitted: &mut bool,
    overrides: &mut config::CliOverrides,
    startup_overrides: &mut config::CliOverrides,
    json_error_mode: bool,
) -> bool {
    match reprobe {
        FastOpenAutoImportReprobe::ImportRequired => true,
        FastOpenAutoImportReprobe::Current => false,
        FastOpenAutoImportReprobe::Pending(state) => {
            if !*pending_merge_warning_emitted {
                emit_pending_sync_merge_warning(&state, json_error_mode);
                *pending_merge_warning_emitted = true;
            }
            force_pending_merge_read_only_overrides(overrides);
            force_pending_merge_read_only_overrides(startup_overrides);
            false
        }
    }
}

fn emit_pending_sync_merge_warning(
    state: &commands::doctor::PendingSyncMergeState,
    json_mode: bool,
) {
    if json_mode {
        let payload = serde_json::json!({
            "level": "warning",
            "code": "sync_merge_pending",
            "message": "Read-only command is proceeding with auto-import and auto-flush disabled because a sync merge is pending",
            "pending_sync_merge": state,
            "remediation": "Run `obr sync --merge` before any tracker mutation."
        });
        eprintln!(
            "{}",
            serde_json::to_string(&payload).unwrap_or_else(|_| payload.to_string())
        );
    } else {
        eprintln!(
            "warning: pending sync merge is {} (phase={}, receipt={}): {}. \
             Read-only command will proceed with automatic sync disabled; run `obr sync --merge` before mutating.",
            state.condition_name(),
            state.phase.as_deref().unwrap_or("unknown"),
            state.receipt_id.as_deref().unwrap_or("unvalidated"),
            state.diagnostic
        );
    }
}

fn emit_pending_sync_merge_inspection_warning(error: &BeadsError, json_mode: bool) {
    if json_mode {
        let payload = serde_json::json!({
            "level": "warning",
            "code": "sync_merge_pending_unknown",
            "message": "Read-only command is proceeding with automatic sync disabled because pending merge state could not be inspected",
            "inspection_error": error.to_string(),
            "remediation": "Run `obr doctor --json` and restore read-only database-family access before mutating."
        });
        eprintln!(
            "{}",
            serde_json::to_string(&payload).unwrap_or_else(|_| payload.to_string())
        );
    } else {
        eprintln!(
            "warning: could not prove that no sync merge is pending ({error}). \
             Read-only command will proceed with automatic sync disabled; do not mutate until `obr doctor` succeeds."
        );
    }
}

/// Determine whether a no-DB command can rewrite the resolved JSONL family.
///
/// This is intentionally separate from [`is_mutating_command`]: explicit sync
/// modes suppress the normal post-command auto-flush, but `--flush-only` and
/// `--merge` still publish JSONL themselves. They must therefore acquire the
/// JSONL-family authority before loading their in-memory snapshot, not only
/// inside the eventual exporter after an ABA window has already opened.
const fn no_db_jsonl_write_intent(cmd: &Commands) -> bool {
    if is_mutating_command(cmd) {
        return true;
    }
    matches!(
        cmd,
        Commands::Sync(args) if args.flush_only || args.merge
    )
}

/// Determine if a command must hold `.write.lock` for its whole execution.
const fn needs_write_lock(cmd: &Commands) -> bool {
    if is_mutating_command(cmd) {
        return true;
    }
    match cmd {
        // Every command in this arm can open or snapshot the SQLite DB family.
        // Serialize them before command execution so `--allow-stale`,
        // `--no-auto-import`, and direct command-local open paths do not bypass
        // the startup lock that protects recovery/schema/default metadata work.
        //
        // Every explicit DB-backed sync mode must open storage inside
        // `sync::execute`.
        // `--flush-only` looks like a "just rewrite JSONL" path but also calls
        // `finalize_export` inside a `with_write_transaction`, updating dirty
        // flags, export hashes, and metadata (jsonl_content_hash,
        // last_export_time, needs_flush). Without the `.write.lock`, a
        // concurrent `obr sync --flush-only` racing with another process's
        // auto-flush (or a second `--flush-only`) can trip fsqlite's
        // concurrent-write deadlock that this lock was specifically added
        // to prevent (issue #243). `--status` only renders status after open,
        // but opening storage can still apply schema/runtime defaults or
        // recover the DB family, so it must also serialize before open.
        // `obr sync --witness` hashes JSONL and returns before opening SQLite, so
        // it also should not block behind DB writers. Bare `obr sync` is invalid
        // and fails validation before storage open, so it should not block on
        // `.write.lock` just to report an argument error.
        // Doctor inspects a live SQLite DB family via snapshot copy + rollback
        // write probe, so it must serialize with writers — merged into this arm
        // (identical body as Sync/Init) to satisfy clippy::match_same_arms.
        Commands::List(_)
        | Commands::Show(_)
        | Commands::Search(_)
        | Commands::Coordination { .. }
        | Commands::Ready(_)
        | Commands::Scheduler(_)
        | Commands::Blocked(_)
        | Commands::Count(_)
        | Commands::Stale(_)
        | Commands::Lint(_)
        | Commands::Stats(_)
        | Commands::Status(_)
        | Commands::Changelog(_)
        | Commands::Graph(_)
        | Commands::Comments(_)
        | Commands::Dep { .. }
        | Commands::Label { .. }
        | Commands::Epic { .. }
        | Commands::Query { .. }
        | Commands::Orphans(_)
        | Commands::Audit { .. }
        | Commands::Info(_)
        | Commands::Where
        | Commands::Init { .. } => true,
        Commands::Doctor(args) => doctor_subcommand_needs_write_lock(args),
        Commands::Sync(args) => sync_mode_opens_storage(args),
        Commands::Config { command } => !matches!(
            command,
            obr::cli::ConfigCommands::Path | obr::cli::ConfigCommands::Edit
        ),
        Commands::History(args) => matches!(
            args.command,
            Some(
                obr::cli::HistoryCommands::Restore { .. } | obr::cli::HistoryCommands::Prune { .. }
            )
        ),
        _ => false,
    }
}

const fn should_auto_import(cmd: &Commands) -> bool {
    match cmd {
        Commands::List(_)
        | Commands::Show(_)
        | Commands::Search(_)
        | Commands::Coordination { .. }
        | Commands::Ready(_)
        | Commands::Scheduler(_)
        | Commands::Blocked(_)
        | Commands::Count(_)
        | Commands::Stale(_)
        | Commands::Lint(_)
        | Commands::Stats(_)
        | Commands::Status(_)
        | Commands::Changelog(_)
        | Commands::Graph(_)
        | Commands::Create(_)
        | Commands::Update(_)
        | Commands::Delete(_)
        | Commands::Close(_)
        | Commands::Reopen(_)
        | Commands::Q(_)
        | Commands::Defer(_)
        | Commands::Undefer(_)
        | Commands::Comments(_)
        | Commands::Dep { .. }
        | Commands::Label { .. }
        | Commands::Epic { .. }
        | Commands::Gate { .. }
        | Commands::Capacity { .. }
        | Commands::Query { .. } => true,

        Commands::Init { .. }
        | Commands::Sync(_)
        | Commands::Doctor(_)
        | Commands::Info(_)
        | Commands::Capabilities(_)
        | Commands::RobotDocs { .. }
        | Commands::Schema(_)
        | Commands::Where
        | Commands::Version(_)
        | Commands::VcsStatus(_)
        | Commands::Completions(_)
        | Commands::Audit { .. }
        | Commands::Orphans(_)
        | Commands::Config { .. }
        | Commands::History(_)
        | Commands::Agents(_) => false,

        #[cfg(feature = "mcp")]
        Commands::Serve(_) => false,
    }
}

const fn supports_read_only_fast_open(cmd: &Commands) -> bool {
    match cmd {
        // `--reconcile --dry-run` is read-only by contract: it plans without
        // opening a write transaction, so it may share the status fast path.
        Commands::Sync(args) => args.status || (args.reconcile && args.dry_run),
        Commands::Stats(_)
        | Commands::Status(_)
        | Commands::Coordination { .. }
        | Commands::List(_)
        | Commands::Show(_)
        | Commands::Search(_)
        | Commands::Ready(_)
        | Commands::Scheduler(_)
        | Commands::Blocked(_)
        | Commands::Count(_)
        | Commands::Stale(_)
        | Commands::Lint(_)
        | Commands::Changelog(_)
        | Commands::Graph(_)
        | Commands::Orphans(obr::cli::OrphansArgs { fix: false, .. })
        | Commands::Comments(obr::cli::CommentsArgs {
            command: None | Some(obr::cli::CommentCommands::List(_)),
            ..
        })
        | Commands::Epic {
            command: obr::cli::EpicCommands::Status(_),
        } => true,
        Commands::Dep { command } => is_read_only_dep_command(command),
        Commands::Label { command } => is_read_only_label_listing(command),
        Commands::Query { command } => is_read_only_query_command(command),
        _ => false,
    }
}

/// Commands whose default freshness contract can use a nonmutating JSONL
/// probe on a current-schema read-only connection, reopening writable storage
/// only after a positive probe has been repeated under database-family
/// authority. This is intentionally narrower than all read-only fast-open
/// commands: activity-bearing stats/status and issue-specific lint retain the
/// ordinary startup path.
const fn supports_auto_import_read_only_probe(cmd: &Commands) -> bool {
    match cmd {
        // Sync never participates in startup auto-import. Both status and the
        // reconcile planner are observational, so their default invocations
        // can use the same current-schema lock-free open without requiring
        // redundant explicit auto-sync opt-outs.
        Commands::Sync(args) => args.status || (args.reconcile && args.dry_run),
        Commands::List(_)
        | Commands::Show(_)
        | Commands::Search(_)
        | Commands::Coordination { .. }
        | Commands::Ready(_)
        | Commands::Scheduler(_)
        | Commands::Blocked(_)
        | Commands::Count(_)
        | Commands::Stale(_)
        | Commands::Changelog(_)
        | Commands::Graph(_)
        | Commands::Comments(obr::cli::CommentsArgs {
            command: None | Some(obr::cli::CommentCommands::List(_)),
            ..
        })
        | Commands::Epic {
            command: obr::cli::EpicCommands::Status(_),
        } => true,
        Commands::Lint(args) => args.ids.is_empty(),
        Commands::Label { command } => is_read_only_label_listing(command),
        Commands::Dep { command } => is_read_only_dep_command(command),
        Commands::Query { command } => is_read_only_query_command(command),
        Commands::Stats(args) | Commands::Status(args) => args.no_activity,
        _ => false,
    }
}

const fn is_read_only_dep_command(command: &obr::cli::DepCommands) -> bool {
    match command {
        obr::cli::DepCommands::List(_)
        | obr::cli::DepCommands::Tree(_)
        | obr::cli::DepCommands::Cycles(_) => true,
        obr::cli::DepCommands::Add(_)
        | obr::cli::DepCommands::Import(_)
        | obr::cli::DepCommands::Remove(_) => false,
    }
}

const fn is_read_only_label_listing(command: &obr::cli::LabelCommands) -> bool {
    match command {
        obr::cli::LabelCommands::ListAll
        | obr::cli::LabelCommands::List(obr::cli::LabelListArgs { issue: None }) => true,
        obr::cli::LabelCommands::Add(_)
        | obr::cli::LabelCommands::Remove(_)
        | obr::cli::LabelCommands::List(_)
        | obr::cli::LabelCommands::Rename(_) => false,
    }
}

const fn is_read_only_query_command(command: &obr::cli::QueryCommands) -> bool {
    match command {
        obr::cli::QueryCommands::Run(_) | obr::cli::QueryCommands::List => true,
        obr::cli::QueryCommands::Save(_) | obr::cli::QueryCommands::Delete(_) => false,
    }
}

fn command_requested_output_format(cmd: &Commands) -> Option<OutputFormat> {
    match cmd {
        Commands::List(args) => args.format,
        Commands::Search(args) => args.filters.format,
        Commands::Show(args) => args.format.map(Into::into),
        Commands::Coordination { command } => match command {
            obr::cli::CoordinationCommands::Status(args) => args.format.map(Into::into),
        },
        Commands::Capabilities(args) => args.format.map(Into::into),
        Commands::RobotDocs { command } => match command {
            obr::cli::RobotDocsCommands::Guide(args) => args.format.map(Into::into),
        },
        Commands::Ready(args) => args.format.map(Into::into),
        Commands::Scheduler(args) => args.format.map(Into::into),
        Commands::Blocked(args) => args.format.map(Into::into),
        Commands::Stats(args) | Commands::Status(args) => args.format.map(Into::into),
        Commands::Schema(args) => args.format.map(Into::into),
        Commands::Dep { command } => match command {
            obr::cli::DepCommands::List(args) => args.format.map(Into::into),
            obr::cli::DepCommands::Tree(_)
            | obr::cli::DepCommands::Add(_)
            | obr::cli::DepCommands::Import(_)
            | obr::cli::DepCommands::Remove(_)
            | obr::cli::DepCommands::Cycles(_) => None,
        },
        Commands::Query { command } => match command {
            obr::cli::QueryCommands::Run(args) => args.filters.format,
            obr::cli::QueryCommands::Save(_)
            | obr::cli::QueryCommands::List
            | obr::cli::QueryCommands::Delete(_) => None,
        },
        _ => None,
    }
}

fn should_render_errors_as_json_with_env(
    cli: &Cli,
    env_output_format: Option<OutputFormat>,
) -> bool {
    cli.json
        || command_requests_robot_json(&cli.command)
        || matches!(
            command_requested_output_format(&cli.command).or(env_output_format),
            Some(OutputFormat::Json | OutputFormat::Toon)
        )
}

fn should_render_errors_as_json(cli: &Cli) -> bool {
    should_render_errors_as_json_with_env(cli, OutputFormat::from_env())
}

/// Whether this invocation should let a closed stdout pipe terminate the
/// process the way every other Unix filter does (`br list | head`), instead
/// of panicking inside `println!` and aborting with SIGABRT under
/// `panic = "abort"` (#434).
///
/// `structured_output` is the same verdict that routes errors to JSON: when
/// it is set, output streams through the JSON/TOON writers, which already
/// classify a broken pipe as a non-error and exit 0, so that contract is
/// preserved. `br serve` keeps `EPIPE` as an error so its stdio transport can
/// shut the server down cooperatively. Everything else prints through bare
/// `println!` sites that cannot classify `EPIPE`, so the kernel's default
/// `SIGPIPE` action is the only disposition that ends them cleanly.
const fn should_restore_default_sigpipe(cli: &Cli, structured_output: bool) -> bool {
    if structured_output {
        return false;
    }
    #[cfg(feature = "mcp")]
    if matches!(cli.command, Commands::Serve(_)) {
        return false;
    }
    #[cfg(not(feature = "mcp"))]
    let _ = cli;
    true
}

const fn should_color_human_errors(
    no_color_flag: bool,
    no_color_env_present: bool,
    stderr_is_terminal: bool,
) -> bool {
    !no_color_flag && !no_color_env_present && stderr_is_terminal
}

fn should_color_human_errors_for_cli(cli: &Cli) -> bool {
    should_color_human_errors(
        cli.no_color,
        std::env::var_os("NO_COLOR").is_some(),
        io::stderr().is_terminal(),
    )
}

/// Handle errors with structured output support.
fn handle_error(err: &BeadsError, json_mode: bool, color_mode: bool) -> ! {
    let structured = StructuredError::from_error(err);
    let exit_code = structured.code.exit_code();

    if json_mode {
        // #336: In `--json` mode, route the structured JSON error envelope to
        // STDOUT (where success JSON already goes) so robot callers read ONE
        // clean, parseable stream. tracing/log lines stay on stderr (see
        // `logging::init_logging`, which writes to `std::io::stderr`), so the
        // stdout JSON is never interleaved with diagnostic noise.
        let json = structured.to_json();
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string())
        );
    } else {
        // Human mode: errors stay on stderr so stdout remains usable for the
        // command's normal (non-error) output and pipelines.
        eprintln!("{}", structured.to_human(color_mode));
    }

    obr::shutdown::exit_process(exit_code);
}

/// Only reachable with `--robot-triage`: the startup lock-failure branch is
/// entered only when the lock was needed, i.e. `--repair` / `--repair-indexes`,
/// and repair-without-triage exits ConcurrencyLost before reaching here. The
/// non-triage JSON payload builder this used to select was therefore dead code
/// with a passing unit test — the defect class this effort exists to remove — so
/// it and the parameter that selected it are gone (obr-fpc).
fn emit_read_only_doctor_write_lock_diagnostic(
    obr_dir: Option<&Path>,
    err: &BeadsError,
    json_mode: bool,
) -> ! {
    let lock_path = obr_dir
        .map(|dir| dir.join(".write.lock"))
        .unwrap_or_else(|| PathBuf::from(".obr/.write.lock"));
    let lock_display = lock_path.display().to_string();
    let remediation =
        format!("`chmod u+w {lock_display}` or remove the file (the next obr call recreates it)");
    let message = format!(
        "{lock_display} is not writable by owner; obr doctor cannot acquire the startup workspace lock for live inspection"
    );
    let exit_code =
        obr::cli::commands::doctor_subsystems::exit_codes::DoctorExitCode::FindingsPresent;

    if json_mode {
        let payload = read_only_doctor_write_lock_triage_payload(
            &lock_path,
            &message,
            &remediation,
            &err.to_string(),
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
        );
    } else {
        eprintln!(
            "obr doctor found an issue before live inspection could start:\n\
             permissions.write_lock: warn\n\
             {message}\n\
             Remediation: {remediation}\n\
             Underlying error: {err}",
        );
    }

    obr::shutdown::exit_process(exit_code.as_i32());
}

fn is_unwritable_write_lock_open_error(lock_path: &Path, err: &BeadsError) -> bool {
    let BeadsError::Config(message) = err else {
        return false;
    };
    message.contains(&write_lock_open_failure_prefix()) && write_lock_lacks_owner_write(lock_path)
}

/// The prefix `src/sync` produces when opening `.write.lock` fails.
///
/// Built from [`obr::sync::WORKSPACE_WRITE_LOCK_ROLE`] rather than spelled out,
/// because this matcher is the only gate on the read-only doctor write-lock
/// diagnostic: when the role was renamed and this literal was not, the gate
/// silently stopped firing and `obr doctor` died on an unwritable lock instead
/// of reporting it. Tests build their fixture errors from the same helper, so a
/// message that no longer matches cannot pass the suite.
fn write_lock_open_failure_prefix() -> String {
    format!("Failed to open {}", obr::sync::WORKSPACE_WRITE_LOCK_ROLE)
}

fn is_write_lock_contention_error(lock_path: &Path, err: &BeadsError) -> bool {
    let BeadsError::Config(message) = err else {
        return false;
    };
    message.contains("Timed out after")
        && message.contains("waiting for write lock")
        && message.contains(lock_path.to_string_lossy().as_ref())
}

/// Only reachable with `--robot-triage`: the startup lock-failure branch is
/// entered only when the lock was needed, i.e. `--repair` / `--repair-indexes`,
/// and repair-without-triage exits ConcurrencyLost before reaching here. The
/// non-triage JSON payload builder this used to select was therefore dead code
/// with a passing unit test — the defect class this effort exists to remove — so
/// it and the parameter that selected it are gone (obr-fpc).
fn emit_read_only_doctor_live_write_lock_diagnostic(
    obr_dir: Option<&Path>,
    err: &BeadsError,
    json_mode: bool,
) -> ! {
    let lock_path = obr_dir
        .map(|dir| dir.join(".write.lock"))
        .unwrap_or_else(|| PathBuf::from(".obr/.write.lock"));
    let lock_display = lock_path.display().to_string();
    let message = format!(
        "Workspace advisory lock at {lock_display} is owned by another process; doctor did not inspect live state"
    );
    let exit_code =
        obr::cli::commands::doctor_subsystems::exit_codes::DoctorExitCode::ConcurrencyLost;

    if json_mode {
        let payload =
            read_only_doctor_live_write_lock_triage_payload(&lock_path, &message, &err.to_string());
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
        );
    } else {
        eprintln!(
            "obr doctor could not begin live inspection:\n\
             write_lock: warn\n\
             {message}\n\
             Wait for the owning process to finish and retry. Do not move or delete the lock inode.\n\
             Underlying error: {err}",
        );
    }

    obr::shutdown::exit_process(exit_code.as_i32());
}

fn read_only_doctor_live_write_lock_triage_payload(
    lock_path: &Path,
    message: &str,
    startup_error: &str,
) -> serde_json::Value {
    read_only_doctor_startup_triage_payload(
        lock_path,
        message,
        startup_error,
        "fm-concurrency_primitives-orphaned-write-lock",
        "P1",
        "live_owner",
        None,
        obr::cli::commands::doctor_subsystems::exit_codes::DoctorExitCode::ConcurrencyLost,
    )
}

fn read_only_doctor_write_lock_triage_payload(
    lock_path: &Path,
    message: &str,
    remediation: &str,
    startup_error: &str,
) -> serde_json::Value {
    read_only_doctor_startup_triage_payload(
        lock_path,
        message,
        startup_error,
        "fm-state_files-orphaned-write-lock",
        "P2",
        "owner_not_writable",
        Some(remediation),
        obr::cli::commands::doctor_subsystems::exit_codes::DoctorExitCode::FindingsPresent,
    )
}

#[allow(clippy::too_many_arguments)]
fn read_only_doctor_startup_triage_payload(
    lock_path: &Path,
    message: &str,
    startup_error: &str,
    finding_id: &str,
    severity: &str,
    reason: &str,
    remediation: Option<&str>,
    exit_code: obr::cli::commands::doctor_subsystems::exit_codes::DoctorExitCode,
) -> serde_json::Value {
    use obr::cli::commands::doctor_subsystems::surface::{TriageFinding, build_triage_envelope};

    let envelope = build_triage_envelope(
        0,
        1,
        0,
        vec![TriageFinding {
            id: finding_id.to_string(),
            severity: severity.to_string(),
            message: message.to_string(),
        }],
    );
    let mut payload = serde_json::to_value(envelope).unwrap_or_else(|serialization_error| {
        serde_json::json!({
            "schema_version": "obr.doctor.triage.v1",
            "summary": "doctor could not begin live inspection",
            "findings": [{
                "id": finding_id,
                "severity": severity,
                "message": message,
            }],
            "actions_planned": [],
            "recommended_command": "obr doctor",
            "capabilities_url": "obr doctor capabilities --format json",
            "robot_docs_command": "obr doctor robot-docs",
            "quick_ref": {"healthy": 0, "warn": 1, "error": 0},
            "serialization_error": serialization_error.to_string(),
        })
    });
    if let Some(object) = payload.as_object_mut() {
        object.insert("ok".to_string(), serde_json::Value::Bool(false));
        object.insert(
            "exit_code".to_string(),
            serde_json::Value::from(exit_code.as_i32()),
        );
        object.insert(
            "code".to_string(),
            serde_json::Value::String(exit_code.as_str().to_string()),
        );
        object.insert(
            "inspection_state".to_string(),
            serde_json::Value::String("not_started".to_string()),
        );
        object.insert(
            "lock_path".to_string(),
            serde_json::Value::String(lock_path.display().to_string()),
        );
        object.insert(
            "reason".to_string(),
            serde_json::Value::String(reason.to_string()),
        );
        object.insert(
            "startup_error".to_string(),
            serde_json::Value::String(startup_error.to_string()),
        );
        if let Some(remediation) = remediation {
            object.insert(
                "remediation".to_string(),
                serde_json::Value::String(remediation.to_string()),
            );
        }
    }
    payload
}

#[cfg(unix)]
fn write_lock_lacks_owner_write(lock_path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(meta) = fs::symlink_metadata(lock_path) else {
        return false;
    };
    meta.is_file() && !meta.file_type().is_symlink() && (meta.permissions().mode() & 0o200) == 0
}

#[cfg(not(unix))]
fn write_lock_lacks_owner_write(_lock_path: &Path) -> bool {
    false
}

#[cfg(not(unix))]
fn lock_mode_octal(_lock_path: &Path) -> Option<String> {
    None
}

fn build_cli_overrides(cli: &Cli) -> config::CliOverrides {
    // `--lock-timeout` deliberately does NOT disable the fast path. The
    // timeout governs how long to wait *when a lock is required*, and a
    // read-only fast open acquires none; on a fast-open miss the fallback
    // still takes the write lock and honors the timeout. Gating on it meant
    // any scripted caller that passed `--lock-timeout` silently lost lock-free
    // reads and serialized every read command behind an exclusive flock,
    // which shows up as spurious lock-timeout failures under parallel reads.
    let read_only_fast_open = !cli.no_db
        && !read_only_fast_open_disabled_for_cli()
        && supports_read_only_fast_open(&cli.command)
        && ((cli.no_auto_import && cli.no_auto_flush)
            || supports_auto_import_read_only_probe(&cli.command));

    let mut overrides = config::CliOverrides::default();
    overrides.db.clone_from(&cli.db);
    overrides.actor.clone_from(&cli.actor);
    // Only set bool overrides when the CLI flag was explicitly provided.
    // Eagerly setting Some(false) would override config-file values with the
    // CLI default, preventing users from setting these via config.
    overrides.json = cli.json.then_some(true);
    overrides.display_color = if cli.no_color { Some(false) } else { None };
    overrides.quiet = cli.quiet.then_some(true);
    overrides.allow_stale = cli.allow_stale.then_some(true);
    overrides.no_db = cli.no_db.then_some(true);
    overrides.no_daemon = cli.no_daemon.then_some(true);
    overrides.no_auto_flush = cli.no_auto_flush.then_some(true);
    overrides.no_auto_import = cli.no_auto_import.then_some(true);
    overrides.lock_timeout = cli.lock_timeout;
    overrides.mark_no_db_write_intent(no_db_jsonl_write_intent(&cli.command));
    overrides.read_only_fast_open = read_only_fast_open;
    overrides
}

#[cfg(not(test))]
fn read_only_fast_open_disabled_for_cli() -> bool {
    std::env::var_os(DISABLE_READ_ONLY_FAST_OPEN_ENV)
        .as_deref()
        .is_some_and(read_only_fast_open_disable_value_is_truthy)
}

#[cfg(test)]
const fn read_only_fast_open_disabled_for_cli() -> bool {
    false
}

fn read_only_fast_open_disable_value_is_truthy(value: &OsStr) -> bool {
    matches!(
        value.to_string_lossy().trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use obr as beads_rust;
    use std::fs;
    use tempfile::TempDir;

    fn make_create_args() -> obr::cli::CreateArgs {
        obr::cli::CreateArgs {
            title: Some("test-title".to_string()),
            title_flag: None,
            type_: None,
            slug: None,
            priority: None,
            description: None,
            description_file: None,
            assignee: None,
            owner: None,
            acceptance_criteria: None,
            agent_context: None,
            labels: Vec::new(),
            parent: None,
            deps: Vec::new(),
            estimate: None,
            due: None,
            defer: None,
            external_ref: None,
            status: None,
            ephemeral: false,
            dry_run: false,
            silent: false,
            file: None,
            agent_name: None,
            harness: None,
            model: None,
        }
    }

    #[test]
    fn parse_global_flags_and_command() {
        let cli = Cli::parse_from(["obr", "--json", "-vv", "list"]);
        assert!(cli.json);
        assert_eq!(cli.verbose, 2);
        assert!(!cli.quiet);
        assert!(matches!(cli.command, Commands::List(_)));
    }

    #[test]
    fn parse_create_title_positional() {
        let cli = Cli::parse_from(["obr", "create", "FixBug"]);
        match cli.command {
            Commands::Create(args) => {
                assert_eq!(args.title.as_deref(), Some("FixBug"));
            }
            other => unreachable!("expected create command, got {other:?}"),
        }
    }

    #[test]
    fn human_error_color_respects_no_color_precedence() {
        assert!(
            should_color_human_errors(false, false, true),
            "interactive stderr should use color when no color controls are set"
        );
        assert!(
            !should_color_human_errors(true, false, true),
            "--no-color must suppress ANSI error output even on a TTY"
        );
        assert!(
            !should_color_human_errors(false, true, true),
            "NO_COLOR must suppress ANSI error output even on a TTY"
        );
        assert!(
            !should_color_human_errors(false, false, false),
            "non-terminal stderr should not receive ANSI error output"
        );
    }

    #[test]
    fn build_overrides_maps_flags() {
        let cli = Cli::parse_from([
            "obr",
            "--json",
            "--no-color",
            "--allow-stale",
            "--no-db",
            "--no-auto-flush",
            "--lock-timeout",
            "2500",
            "list",
        ]);
        let overrides = build_cli_overrides(&cli);
        assert_eq!(overrides.json, Some(true));
        assert_eq!(overrides.display_color, Some(false));
        assert_eq!(overrides.allow_stale, Some(true));
        assert_eq!(overrides.no_db, Some(true));
        assert_eq!(overrides.no_auto_flush, Some(true));
        assert_eq!(overrides.lock_timeout, Some(2500));
    }

    #[test]
    fn build_overrides_omits_absent_startup_bool_flags() {
        let cli = Cli::parse_from(["obr", "list"]);
        let overrides = build_cli_overrides(&cli);

        // Absent CLI bool flags must not produce Some(false) overrides — that
        // would silently clobber any config-file value (e.g. `sync.auto_flush:
        // false` would be ignored because the CLI's default `false` wins).
        assert_eq!(overrides.json, None);
        assert_eq!(overrides.quiet, None);
        assert_eq!(overrides.no_db, None);
        assert_eq!(overrides.no_daemon, None);
        assert_eq!(overrides.no_auto_flush, None);
        assert_eq!(overrides.no_auto_import, None);
        assert_eq!(overrides.allow_stale, None);
    }

    #[test]
    fn doctor_repair_startup_write_lock_fails_fast_by_default() {
        let ctx = StartupContext::empty(config::CliOverrides::default());
        let doctor_repair = Cli::parse_from(["obr", "doctor", "--repair", "--dry-run"]);
        let doctor_read_only = Cli::parse_from(["obr", "doctor"]);

        assert_eq!(
            ctx.startup_write_lock_timeout(&doctor_repair.command),
            Some(0),
            "doctor repair should try-lock by default so contention returns concurrency_lost quickly"
        );
        assert_eq!(
            ctx.startup_write_lock_timeout(&doctor_read_only.command),
            Some(obr::sync::default_write_lock_timeout_ms()),
            "plain doctor should keep the normal startup lock timeout"
        );
    }

    #[test]
    fn doctor_repair_startup_write_lock_honors_explicit_timeout() {
        let cli = Cli::parse_from([
            "obr",
            "--lock-timeout",
            "2500",
            "doctor",
            "--repair",
            "--dry-run",
        ]);
        let overrides = build_cli_overrides(&cli);
        let mut ctx = StartupContext::empty(overrides.clone());
        ctx.config = Some(overrides.as_layer());

        assert_eq!(ctx.startup_write_lock_timeout(&cli.command), Some(2500));
    }

    #[test]
    fn read_only_doctor_write_lock_diagnostic_only_catches_unwritable_regular_file() {
        let temp = TempDir::new().expect("tempdir");
        let obr_dir = temp.path().join(".beads");
        fs::create_dir_all(&obr_dir).expect("mkdir");
        let lock = obr_dir.join(".write.lock");
        fs::write(&lock, b"").expect("write lock");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o444)).expect("chmod");
        }
        // Built from the same helper the matcher uses, and thus from the same
        // role constant `src/sync` reports with. Spelling this literally is what
        // let 251b501b rename the role and leave this test green while the
        // diagnostic it guards was dead.
        let open_err = BeadsError::Config(format!(
            "{} at {}: Permission denied",
            write_lock_open_failure_prefix(),
            lock.display()
        ));
        let timeout_err = BeadsError::Config(format!(
            "Timed out after 1ms waiting for write lock at {}",
            lock.display()
        ));

        #[cfg(unix)]
        assert!(is_unwritable_write_lock_open_error(&lock, &open_err));
        #[cfg(not(unix))]
        assert!(!is_unwritable_write_lock_open_error(&lock, &open_err));
        assert!(
            !is_unwritable_write_lock_open_error(&lock, &timeout_err),
            "lock contention must not be reported as permissions.write_lock"
        );
        assert!(
            !is_unwritable_write_lock_open_error(&obr_dir.join("missing.lock"), &open_err),
            "missing lock path must fall back to the original startup error"
        );

        let directory_lock = obr_dir.join("directory.lock");
        fs::create_dir(&directory_lock).expect("directory lock");
        assert!(
            !is_unwritable_write_lock_open_error(&directory_lock, &open_err),
            "non-file lock path belongs to the original startup error, not permissions.write_lock"
        );
    }

    #[test]
    fn read_only_doctor_live_write_lock_triage_payload_preserves_v1_contract() {
        let lock = PathBuf::from("/workspace/.beads/.write.lock");
        let payload = read_only_doctor_live_write_lock_triage_payload(
            &lock,
            "lock is owned",
            "Timed out after 1ms waiting for write lock",
        );

        assert_eq!(payload["schema_version"], "obr.doctor.triage.v1");
        assert!(payload["summary"].is_string());
        assert!(payload["findings"].is_array());
        assert!(payload["actions_planned"].is_array());
        assert!(payload["recommended_command"].is_string());
        assert!(payload["capabilities_url"].is_string());
        assert!(payload["robot_docs_command"].is_string());
        assert_eq!(payload["quick_ref"]["healthy"], 0);
        assert_eq!(payload["quick_ref"]["warn"], 1);
        assert_eq!(payload["quick_ref"]["error"], 0);
        assert_eq!(
            payload["findings"][0]["id"],
            "fm-concurrency_primitives-orphaned-write-lock"
        );
        assert_eq!(payload["exit_code"], 5);
        assert_eq!(payload["code"], "concurrency_lost");
        assert_eq!(payload["inspection_state"], "not_started");
        assert_eq!(payload["reason"], "live_owner");
        assert!(
            payload.get("workspace_health").is_none(),
            "an uninspected workspace must not receive a health classification"
        );
    }

    #[test]
    fn read_only_doctor_unwritable_write_lock_triage_payload_preserves_v1_contract() {
        let lock = PathBuf::from("/workspace/.beads/.write.lock");
        let payload = read_only_doctor_write_lock_triage_payload(
            &lock,
            "lock is not writable",
            "chmod u+w /workspace/.beads/.write.lock",
            "Failed to open write lock: Permission denied",
        );

        assert_eq!(payload["schema_version"], "obr.doctor.triage.v1");
        assert!(payload["summary"].is_string());
        assert!(payload["findings"].is_array());
        assert!(payload["actions_planned"].is_array());
        assert!(payload["recommended_command"].is_string());
        assert!(payload["capabilities_url"].is_string());
        assert!(payload["robot_docs_command"].is_string());
        assert_eq!(payload["quick_ref"]["healthy"], 0);
        assert_eq!(payload["quick_ref"]["warn"], 1);
        assert_eq!(payload["quick_ref"]["error"], 0);
        assert_eq!(
            payload["findings"][0]["id"],
            "fm-state_files-orphaned-write-lock"
        );
        assert_eq!(payload["findings"][0]["severity"], "P2");
        assert_eq!(payload["exit_code"], 1);
        assert_eq!(payload["code"], "findings_present");
        assert_eq!(payload["inspection_state"], "not_started");
        assert_eq!(payload["reason"], "owner_not_writable");
        assert!(
            payload["remediation"]
                .as_str()
                .is_some_and(|message| message.contains("chmod u+w"))
        );
        assert!(
            payload.get("workspace_health").is_none(),
            "an uninspected workspace must not receive a health classification"
        );
    }

    #[test]
    fn write_lock_contention_detection_is_path_scoped() {
        let lock = PathBuf::from("/workspace/.beads/.write.lock");
        // Built by the real producer, not hand-spelled. This test is the twin of
        // the one that let the "Failed to open write lock" matcher rot: it agreed
        // with its own literal while src/sync had moved on. If the timeout message
        // changes shape now, this fails instead of quietly passing.
        let timeout = obr::sync::write_lock_timeout_error(
            obr::sync::WORKSPACE_WRITE_LOCK_ROLE,
            &lock.display().to_string(),
            1,
        );
        let other_lock = PathBuf::from("/other/.beads/.write.lock");
        let open_error = BeadsError::Config(format!(
            "{} at {}: Permission denied",
            write_lock_open_failure_prefix(),
            lock.display()
        ));

        assert!(is_write_lock_contention_error(&lock, &timeout));
        assert!(!is_write_lock_contention_error(&other_lock, &timeout));
        assert!(!is_write_lock_contention_error(&lock, &open_error));
    }

    #[test]
    fn read_only_fast_open_supports_explicit_opt_out_and_default_safe_probe() {
        let list = Cli::parse_from(["obr", "list"]);
        assert!(build_cli_overrides(&list).read_only_fast_open);

        let list_with_lock_timeout = Cli::parse_from(["obr", "--lock-timeout", "50", "list"]);
        assert!(build_cli_overrides(&list_with_lock_timeout).read_only_fast_open);

        let stats = Cli::parse_from(["obr", "stats"]);
        assert!(!build_cli_overrides(&stats).read_only_fast_open);

        let stats_no_auto =
            Cli::parse_from(["obr", "--no-auto-import", "--no-auto-flush", "stats"]);
        assert!(build_cli_overrides(&stats_no_auto).read_only_fast_open);

        let stats_no_activity = Cli::parse_from(["obr", "stats", "--no-activity"]);
        assert!(build_cli_overrides(&stats_no_activity).read_only_fast_open);

        let status = Cli::parse_from(["obr", "status"]);
        assert!(!build_cli_overrides(&status).read_only_fast_open);

        let status_no_auto =
            Cli::parse_from(["obr", "--no-auto-import", "--no-auto-flush", "status"]);
        assert!(build_cli_overrides(&status_no_auto).read_only_fast_open);

        let status_no_activity = Cli::parse_from(["obr", "status", "--no-activity"]);
        assert!(build_cli_overrides(&status_no_activity).read_only_fast_open);

        let sync_status = Cli::parse_from(["obr", "sync", "--status"]);
        assert!(build_cli_overrides(&sync_status).read_only_fast_open);

        let sync_reconcile_dry_run = Cli::parse_from(["obr", "sync", "--reconcile", "--dry-run"]);
        assert!(
            build_cli_overrides(&sync_reconcile_dry_run).read_only_fast_open,
            "the reconcile planner is observational and must not wait behind the writer lock"
        );

        let sync_reconcile_apply = Cli::parse_from(["obr", "sync", "--reconcile"]);
        assert!(!build_cli_overrides(&sync_reconcile_apply).read_only_fast_open);

        let sync_flush = Cli::parse_from(["obr", "sync", "--flush-only"]);
        assert!(!build_cli_overrides(&sync_flush).read_only_fast_open);

        let sync_import = Cli::parse_from(["obr", "sync", "--import-only"]);
        assert!(!build_cli_overrides(&sync_import).read_only_fast_open);

        let ready = Cli::parse_from(["obr", "--no-auto-import", "--no-auto-flush", "ready"]);
        assert!(build_cli_overrides(&ready).read_only_fast_open);
    }

    #[test]
    fn read_only_fast_open_classifies_auxiliary_read_and_write_commands() {
        let changelog = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "changelog",
            "--json",
        ]);
        assert!(build_cli_overrides(&changelog).read_only_fast_open);

        let comments_list = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "comments",
            "list",
            "bd-abc",
        ]);
        assert!(build_cli_overrides(&comments_list).read_only_fast_open);

        let comments_shorthand = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "comments",
            "bd-abc",
        ]);
        assert!(build_cli_overrides(&comments_shorthand).read_only_fast_open);

        let label_list_all = Cli::parse_from(["obr", "label", "list-all"]);
        assert!(build_cli_overrides(&label_list_all).read_only_fast_open);

        let label_list_unique = Cli::parse_from(["obr", "label", "list"]);
        assert!(build_cli_overrides(&label_list_unique).read_only_fast_open);

        let count = Cli::parse_from(["obr", "count", "--by", "status"]);
        assert!(build_cli_overrides(&count).read_only_fast_open);

        let label_list_issue = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "label",
            "list",
            "bd-abc",
        ]);
        assert!(!build_cli_overrides(&label_list_issue).read_only_fast_open);

        let comments_no_auto_import =
            Cli::parse_from(["obr", "--no-auto-import", "comments", "list", "bd-abc"]);
        assert!(build_cli_overrides(&comments_no_auto_import).read_only_fast_open);

        let mutating = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "create",
            "write path",
        ]);
        assert!(!build_cli_overrides(&mutating).read_only_fast_open);

        let label_add = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "label",
            "add",
            "bd-abc",
            "--label",
            "bug",
        ]);
        assert!(!build_cli_overrides(&label_add).read_only_fast_open);

        let comments_add = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "comments",
            "add",
            "bd-abc",
            "write path",
        ]);
        assert!(!build_cli_overrides(&comments_add).read_only_fast_open);
    }

    #[test]
    fn read_only_fast_open_disable_env_parser_is_conservative() {
        for value in ["1", "true", "TRUE", " yes ", "on"] {
            assert!(
                read_only_fast_open_disable_value_is_truthy(OsStr::new(value)),
                "{value:?} should disable read-only fast-open"
            );
        }

        for value in ["", "0", "false", "off", "no", "maybe"] {
            assert!(
                !read_only_fast_open_disable_value_is_truthy(OsStr::new(value)),
                "{value:?} should not disable read-only fast-open"
            );
        }
    }

    #[test]
    fn default_read_commands_fast_open_only_for_safe_auto_import_probe() {
        let ready = Cli::parse_from(["obr", "ready"]);
        assert!(build_cli_overrides(&ready).read_only_fast_open);

        let blocked = Cli::parse_from(["obr", "blocked"]);
        assert!(build_cli_overrides(&blocked).read_only_fast_open);

        let show = Cli::parse_from(["obr", "show", "br-123"]);
        assert!(build_cli_overrides(&show).read_only_fast_open);

        let comments_list = Cli::parse_from(["obr", "comments", "list", "br-123"]);
        assert!(build_cli_overrides(&comments_list).read_only_fast_open);

        let search = Cli::parse_from(["obr", "search", "needle"]);
        assert!(build_cli_overrides(&search).read_only_fast_open);

        let stale = Cli::parse_from(["obr", "stale"]);
        assert!(build_cli_overrides(&stale).read_only_fast_open);

        let lint = Cli::parse_from(["obr", "lint"]);
        assert!(build_cli_overrides(&lint).read_only_fast_open);

        let lint_issue = Cli::parse_from(["obr", "lint", "br-123"]);
        assert!(!build_cli_overrides(&lint_issue).read_only_fast_open);

        let changelog = Cli::parse_from(["obr", "changelog"]);
        assert!(build_cli_overrides(&changelog).read_only_fast_open);

        let graph = Cli::parse_from(["obr", "graph", "--all"]);
        assert!(build_cli_overrides(&graph).read_only_fast_open);

        let orphans = Cli::parse_from(["obr", "orphans"]);
        assert!(
            !build_cli_overrides(&orphans).read_only_fast_open,
            "bare orphans owns a command-local auto-import and cannot receive preopened fast storage"
        );

        let epic_status = Cli::parse_from(["obr", "epic", "status"]);
        assert!(build_cli_overrides(&epic_status).read_only_fast_open);

        let dep_tree = Cli::parse_from(["obr", "dep", "tree", "br-123"]);
        assert!(build_cli_overrides(&dep_tree).read_only_fast_open);

        let dep_list = Cli::parse_from(["obr", "dep", "list", "br-123"]);
        assert!(build_cli_overrides(&dep_list).read_only_fast_open);

        let dep_cycles = Cli::parse_from(["obr", "dep", "cycles"]);
        assert!(build_cli_overrides(&dep_cycles).read_only_fast_open);

        let query_run = Cli::parse_from(["obr", "query", "run", "mine", "--format", "json"]);
        assert!(build_cli_overrides(&query_run).read_only_fast_open);

        let query_list = Cli::parse_from(["obr", "query", "list"]);
        assert!(build_cli_overrides(&query_list).read_only_fast_open);
    }

    #[test]
    fn read_only_fast_open_covers_read_only_query_commands() {
        let query_run = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "query",
            "run",
            "mine",
            "--format",
            "json",
        ]);
        assert!(build_cli_overrides(&query_run).read_only_fast_open);

        let query_list = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "query",
            "list",
        ]);
        assert!(build_cli_overrides(&query_list).read_only_fast_open);

        let no_auto_import_only =
            Cli::parse_from(["obr", "--no-auto-import", "query", "run", "mine"]);
        assert!(build_cli_overrides(&no_auto_import_only).read_only_fast_open);

        let query_save = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "query",
            "save",
            "mine",
            "--status",
            "open",
        ]);
        assert!(!build_cli_overrides(&query_save).read_only_fast_open);

        let query_delete = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "query",
            "delete",
            "mine",
        ]);
        assert!(!build_cli_overrides(&query_delete).read_only_fast_open);
    }

    #[test]
    fn read_only_fast_open_covers_lint_command() {
        let lint = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "lint",
            "--json",
        ]);
        assert!(build_cli_overrides(&lint).read_only_fast_open);
    }

    #[test]
    fn read_only_fast_open_covers_epic_status_only() {
        let status = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "epic",
            "status",
        ]);
        assert!(build_cli_overrides(&status).read_only_fast_open);

        let close_eligible = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "epic",
            "close-eligible",
            "--dry-run",
        ]);
        assert!(!build_cli_overrides(&close_eligible).read_only_fast_open);
    }

    #[test]
    fn read_only_fast_open_covers_graph_and_read_only_dep_commands() {
        let dep_tree = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "dep",
            "tree",
            "bd-abc",
        ]);
        assert!(build_cli_overrides(&dep_tree).read_only_fast_open);

        let dep_cycles = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "dep",
            "cycles",
        ]);
        assert!(build_cli_overrides(&dep_cycles).read_only_fast_open);

        let graph_all = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "graph",
            "--all",
        ]);
        assert!(build_cli_overrides(&graph_all).read_only_fast_open);

        let dep_add = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "dep",
            "add",
            "bd-abc",
            "bd-def",
        ]);
        assert!(!build_cli_overrides(&dep_add).read_only_fast_open);

        let dep_import = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "dep",
            "import",
            "edges.jsonl",
        ]);
        assert!(!build_cli_overrides(&dep_import).read_only_fast_open);
    }

    #[test]
    fn read_only_fast_open_covers_non_fix_orphans_scan() {
        let orphans = Cli::parse_from(["obr", "--no-auto-import", "--no-auto-flush", "orphans"]);
        assert!(build_cli_overrides(&orphans).read_only_fast_open);

        let orphans_json = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "orphans",
            "--json",
        ]);
        assert!(build_cli_overrides(&orphans_json).read_only_fast_open);

        let orphans_fix = Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "orphans",
            "--fix",
        ]);
        assert!(!build_cli_overrides(&orphans_fix).read_only_fast_open);
    }

    #[test]
    fn read_only_fast_open_defers_startup_write_lock_until_fallback() {
        assert!(
            !should_acquire_startup_write_lock(true, false, true),
            "read-only fast-open commands try the current-schema read-only DB before joining the writer lock path"
        );
        assert!(
            !should_acquire_startup_write_lock(true, true, true),
            "auto-import probes can use read-only fast-open first; fast-open misses reacquire the lock before writable fallback"
        );
        assert!(
            should_acquire_startup_write_lock(false, true, false),
            "non-fast-open DB-family commands must keep the startup lock"
        );
    }

    #[test]
    fn initialized_non_database_command_skips_startup_authority_and_pending_gate() {
        let command = Cli::parse_from(["obr", "completions", "bash"]).command;

        assert!(
            !needs_write_lock(&command),
            "completion generation never opens the workspace database"
        );
        assert!(
            !should_acquire_startup_write_lock(
                needs_write_lock(&command),
                should_preopen_storage(true, false),
                false,
            ),
            "an initialized cwd must not make a non-database command enter the live DB gate"
        );
    }

    #[test]
    fn read_only_no_db_command_remains_database_lock_free() {
        let command = Cli::parse_from(["obr", "--no-db", "list"]).command;
        let jsonl_write = no_db_jsonl_write_intent(&command);

        assert!(
            !jsonl_write,
            "read-only list must not claim JSONL write intent"
        );
        assert!(
            !startup_database_authority_required(
                true,
                should_acquire_startup_write_lock(needs_write_lock(&command), false, false),
                jsonl_write,
                false,
            ),
            "read-only no-DB commands must not join the database writer lock path"
        );
    }

    #[test]
    fn file_only_pending_mutation_still_acquires_database_authority_for_gate() {
        let command = Cli::parse_from(["obr", "config", "edit"]).command;
        assert!(
            !needs_write_lock(&command),
            "config edit does not ordinarily open the database"
        );
        let gate_required =
            pending_merge_startup_disposition(&command) == PendingMergeStartupDisposition::Refuse;
        assert!(gate_required, "config edit can redirect merge-owned paths");
        for no_db in [false, true] {
            assert!(
                startup_database_authority_required(no_db, false, false, gate_required),
                "pending-state refusal must acquire authority even when no_db={no_db}"
            );
        }
    }

    /// A main database file that is missing *with no surviving sidecar* is the
    /// one shape that genuinely cannot hide a receipt. This fixture creates no
    /// sidecars at all, so it never reaches the sidecar branch; the broader
    /// claim -- that any missing main file is enough -- is false and is covered
    /// by `no_db_write_inspector_refuses_missing_database_whose_wal_still_holds_frames`.
    #[test]
    fn no_db_write_inspector_allows_exact_missing_database_without_creating_it() {
        let temp = TempDir::new().unwrap();
        let obr_dir = temp.path().join(".beads");
        fs::create_dir_all(&obr_dir).unwrap();
        let db_path = obr_dir.join("beads.db");
        let jsonl_path = obr_dir.join("issues.jsonl");
        let authority = Arc::new(
            obr::sync::blocking_database_family_write_lock_with_timeout(
                &obr_dir,
                &db_path,
                Some(1_000),
            )
            .unwrap(),
        );

        assert!(
            inspect_pending_sync_merge_for_startup_under_authority(&db_path, &authority)
                .unwrap()
                .is_none(),
            "a missing DB with no surviving sidecar cannot contain a pending receipt"
        );
        assert!(
            !db_path.exists(),
            "inspection must not initialize a database in genuine no-DB mode"
        );

        fs::write(&jsonl_path, b"{\"id\":\"br-no-db\"}\n").unwrap();
        assert_eq!(
            fs::read(&jsonl_path).unwrap(),
            b"{\"id\":\"br-no-db\"}\n",
            "missing-DB classification must permit ordinary no-DB JSONL work"
        );
        assert!(!db_path.exists());
    }

    #[test]
    fn startup_advisory_allows_exact_missing_database() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("beads.db");

        assert!(
            inspect_pending_sync_merge_for_startup(&db_path)
                .unwrap()
                .is_none(),
            "a JSONL-only checkout must be allowed to initialize its database"
        );
        assert!(
            !db_path.exists(),
            "advisory inspection must remain read-only"
        );
    }

    /// The no-DB JSONL gate must not read a missing main database file as
    /// "no merge is pending". `bind_database_inode_for_mutation` stats only
    /// the main file, so an orphaned WAL left by a process killed between
    /// COMMIT and the `Drop` checkpoint can still hold the committed merge
    /// receipt. Classifying that as absent lets this gate publish pre-merge
    /// JSONL over merged truth.
    #[test]
    fn no_db_write_inspector_refuses_missing_database_whose_wal_still_holds_frames() {
        // 32-byte WAL header + one 4120-byte frame (24-byte frame header +
        // 4096-byte page). Built byte-wise: killing a live writer is a race.
        for (label, wal_len) in [("one frame", 32 + 24 + 4096), ("one byte past header", 33)] {
            let temp = TempDir::new().unwrap();
            let obr_dir = temp.path().join(".obr");
            fs::create_dir_all(&obr_dir).unwrap();
            let db_path = obr_dir.join("obr.db");
            let jsonl_path = obr_dir.join("issues.jsonl");
            let wal_path = obr_dir.join("obr.db-wal");
            fs::write(&wal_path, vec![0_u8; wal_len]).unwrap();
            fs::write(&jsonl_path, b"{\"id\":\"br-pre-merge\"}\n").unwrap();
            let wal_before = fs::read(&wal_path).unwrap();
            let jsonl_before = fs::read(&jsonl_path).unwrap();
            let authority = Arc::new(
                obr::sync::blocking_database_family_write_lock_with_timeout(
                    &obr_dir,
                    &db_path,
                    Some(1_000),
                )
                .unwrap(),
            );

            let err = inspect_pending_sync_merge_for_startup_under_authority(&db_path, &authority)
                .expect_err(&format!(
                    "{label}: an orphan WAL may hold a committed receipt"
                ));

            assert!(
                err.to_string().contains("unknown"),
                "{label}: refusal must name the state as unknown: {err}"
            );
            assert_eq!(
                fs::read(&wal_path).unwrap(),
                wal_before,
                "{label}: refusal must not touch the WAL"
            );
            assert_eq!(
                fs::read(&jsonl_path).unwrap(),
                jsonl_before,
                "{label}: refusal must not publish JSONL"
            );
            assert!(
                !db_path.exists(),
                "{label}: refusal must not create a database"
            );
        }
    }

    /// The counterpart: with no main file and no sidecar that can carry
    /// committed bytes, there is nothing a receipt could hide in, so the
    /// no-DB path must proceed rather than brick the workspace.
    #[test]
    fn no_db_write_inspector_allows_missing_database_with_only_spent_sidecars() {
        for (label, suffix, len) in [
            ("header-only wal", "-wal", 32_usize),
            ("zero-length wal", "-wal", 0),
            ("zero-length journal", "-journal", 0),
            ("populated shm without a wal", "-shm", 32_768),
        ] {
            let temp = TempDir::new().unwrap();
            let obr_dir = temp.path().join(".obr");
            fs::create_dir_all(&obr_dir).unwrap();
            let db_path = obr_dir.join("obr.db");
            fs::write(obr_dir.join(format!("obr.db{suffix}")), vec![0_u8; len]).unwrap();
            let authority = Arc::new(
                obr::sync::blocking_database_family_write_lock_with_timeout(
                    &obr_dir,
                    &db_path,
                    Some(1_000),
                )
                .unwrap(),
            );

            assert!(
                inspect_pending_sync_merge_for_startup_under_authority(&db_path, &authority)
                    .unwrap_or_else(|error| panic!("{label} must be inspectable: {error}"))
                    .is_none(),
                "{label}: no sidecar here can hold a committed receipt"
            );
            assert!(
                !db_path.exists(),
                "{label}: inspection must not create a database"
            );
        }
    }

    #[test]
    fn fast_open_import_reprobe_rechecks_freshness_and_pending_receipt_under_authority() {
        let temp = TempDir::new().expect("tempdir");
        let beads_dir = temp.path().join(".beads");
        fs::create_dir_all(&beads_dir).expect("create beads dir");

        let bootstrap = config::open_storage_with_cli(&beads_dir, &config::CliOverrides::default())
            .expect("bootstrap storage");
        let paths = bootstrap.paths.clone();
        drop(bootstrap);

        let fast_overrides = build_cli_overrides(&Cli::parse_from(["br", "ready"]));
        let mut fast_storage =
            Some(config::open_storage_with_cli(&beads_dir, &fast_overrides).expect("fast storage"));
        let authority = Arc::new(
            beads_rust::sync::blocking_database_family_write_lock_with_timeout(
                &beads_dir,
                &paths.db_path,
                Some(1_000),
            )
            .expect("database authority"),
        );
        assert!(matches!(
            reopen_and_reprobe_fast_open_auto_import_under_authority(
                &mut fast_storage,
                &paths,
                &fast_overrides,
                &authority,
                false,
            )
            .expect("current reprobe"),
            FastOpenAutoImportReprobe::Current
        ));

        fs::write(&paths.jsonl_path, b"{\"id\":\"br-new\"}\n").expect("write newer JSONL");
        assert!(matches!(
            reopen_and_reprobe_fast_open_auto_import_under_authority(
                &mut fast_storage,
                &paths,
                &fast_overrides,
                &authority,
                false,
            )
            .expect("stale reprobe"),
            FastOpenAutoImportReprobe::ImportRequired
        ));
        drop(authority);
        drop(fast_storage);

        let mut writable =
            config::open_storage_with_cli(&beads_dir, &config::CliOverrides::default())
                .expect("writable storage");
        writable
            .storage
            .set_metadata("sync_merge_pending_v1", "legacy-receipt")
            .expect("plant pending receipt");
        drop(writable);

        let mut fast_storage =
            Some(config::open_storage_with_cli(&beads_dir, &fast_overrides).expect("fast storage"));
        let authority = Arc::new(
            beads_rust::sync::blocking_database_family_write_lock_with_timeout(
                &beads_dir,
                &paths.db_path,
                Some(1_000),
            )
            .expect("database authority"),
        );
        let pending_reprobe = reopen_and_reprobe_fast_open_auto_import_under_authority(
            &mut fast_storage,
            &paths,
            &fast_overrides,
            &authority,
            false,
        )
        .expect("pending reprobe");
        assert!(matches!(
            &pending_reprobe,
            FastOpenAutoImportReprobe::Pending(_)
        ));

        let mut dispatch_overrides = fast_overrides.clone();
        dispatch_overrides.read_only_fast_open = false;
        let mut pending_ctx = StartupContext::empty(fast_overrides.clone());
        pending_ctx.overrides.read_only_fast_open = false;
        let mut warning_emitted = false;
        let should_attempt_auto_import = apply_fast_open_auto_import_reprobe(
            pending_reprobe,
            &mut warning_emitted,
            &mut dispatch_overrides,
            &mut pending_ctx.overrides,
            false,
        );

        assert!(!should_attempt_auto_import);
        assert!(warning_emitted);
        for forced in [&dispatch_overrides, &pending_ctx.overrides] {
            assert_eq!(forced.no_auto_import, Some(true));
            assert_eq!(forced.no_auto_flush, Some(true));
            assert!(forced.read_only_fast_open);
        }
    }

    #[test]
    #[cfg(unix)]
    fn fast_open_import_reprobe_reopens_the_canonical_database_inode() {
        let temp = TempDir::new().expect("tempdir");
        let beads_dir = temp.path().join(".beads");
        fs::create_dir_all(&beads_dir).expect("create beads dir");

        let mut bootstrap =
            config::open_storage_with_cli(&beads_dir, &config::CliOverrides::default())
                .expect("bootstrap storage");
        bootstrap
            .storage
            .set_metadata("fast_open_inode_marker", "old")
            .expect("mark original database");
        let paths = bootstrap.paths.clone();
        drop(bootstrap);

        let fast_overrides = build_cli_overrides(&Cli::parse_from(["br", "ready"]));
        let mut fast_storage = Some(
            config::open_storage_with_cli(&beads_dir, &fast_overrides)
                .expect("open original database read-only"),
        );

        let displaced_path = beads_dir.join("beads.displaced.db");
        fs::rename(&paths.db_path, &displaced_path).expect("displace original database");
        let mut replacement = beads_rust::storage::SqliteStorage::open(&paths.db_path)
            .expect("create canonical replacement");
        replacement
            .set_metadata("fast_open_inode_marker", "replacement")
            .expect("mark replacement database");
        drop(replacement);

        let authority = Arc::new(
            beads_rust::sync::blocking_database_family_write_lock_with_timeout(
                &beads_dir,
                &paths.db_path,
                Some(1_000),
            )
            .expect("database authority"),
        );
        assert!(matches!(
            reopen_and_reprobe_fast_open_auto_import_under_authority(
                &mut fast_storage,
                &paths,
                &fast_overrides,
                &authority,
                false,
            )
            .expect("canonical reprobe"),
            FastOpenAutoImportReprobe::Current
        ));
        assert_eq!(
            fast_storage
                .as_ref()
                .expect("canonical storage retained")
                .storage
                .get_metadata("fast_open_inode_marker")
                .expect("read canonical marker")
                .as_deref(),
            Some("replacement"),
            "the protected reprobe must replace the pre-lock orphaned handle"
        );
    }

    #[test]
    fn fast_open_import_reprobe_retains_frozen_jsonl_route_during_metadata_drift() {
        let temp = TempDir::new().expect("tempdir");
        let beads_dir = temp.path().join(".beads");
        fs::create_dir_all(&beads_dir).expect("create beads dir");
        let metadata_path = beads_dir.join("metadata.json");
        fs::write(
            &metadata_path,
            r#"{"database":"beads.db","jsonl_export":"first.jsonl"}"#,
        )
        .expect("write initial metadata");

        let bootstrap = config::open_storage_with_cli(&beads_dir, &config::CliOverrides::default())
            .expect("bootstrap storage");
        let paths = bootstrap.paths.clone();
        drop(bootstrap);
        fs::write(&paths.jsonl_path, b"{\"id\":\"br-route-drift\"}\n")
            .expect("write first routed JSONL");

        let fast_overrides = build_cli_overrides(&Cli::parse_from(["br", "ready"]));
        let mut fast_storage =
            Some(config::open_storage_with_cli(&beads_dir, &fast_overrides).expect("fast storage"));
        let authority = Arc::new(
            beads_rust::sync::blocking_database_family_write_lock_with_timeout(
                &beads_dir,
                &paths.db_path,
                Some(1_000),
            )
            .expect("database authority"),
        );
        fs::write(
            &metadata_path,
            r#"{"database":"beads.db","jsonl_export":"second.jsonl"}"#,
        )
        .expect("rewrite metadata route");

        let reprobe = reopen_and_reprobe_fast_open_auto_import_under_authority(
            &mut fast_storage,
            &paths,
            &fast_overrides,
            &authority,
            false,
        )
        .expect("protected reopen must use the frozen startup snapshot");
        assert!(matches!(reprobe, FastOpenAutoImportReprobe::ImportRequired));
        assert!(
            fast_storage
                .as_ref()
                .is_some_and(|result| result.paths == paths),
            "metadata drift must not reroute the protected storage reopen"
        );
        assert!(!beads_dir.join("second.jsonl").exists());
    }

    #[test]
    fn fast_open_import_reprobe_propagates_jsonl_probe_errors() {
        let temp = TempDir::new().expect("tempdir");
        let beads_dir = temp.path().join(".beads");
        fs::create_dir_all(&beads_dir).expect("create beads dir");

        let bootstrap = config::open_storage_with_cli(&beads_dir, &config::CliOverrides::default())
            .expect("bootstrap storage");
        let paths = bootstrap.paths.clone();
        drop(bootstrap);
        fs::create_dir(&paths.jsonl_path).expect("plant non-regular JSONL path");

        let fast_overrides = build_cli_overrides(&Cli::parse_from(["br", "ready"]));
        let mut fast_storage =
            Some(config::open_storage_with_cli(&beads_dir, &fast_overrides).expect("fast storage"));
        let authority = Arc::new(
            beads_rust::sync::blocking_database_family_write_lock_with_timeout(
                &beads_dir,
                &paths.db_path,
                Some(1_000),
            )
            .expect("database authority"),
        );

        let result = reopen_and_reprobe_fast_open_auto_import_under_authority(
            &mut fast_storage,
            &paths,
            &fast_overrides,
            &authority,
            false,
        );
        assert!(
            result.is_err(),
            "a non-regular JSONL path must remain an error after authority acquisition"
        );
    }

    #[test]
    fn no_db_write_inspector_refuses_legacy_and_malformed_receipts_without_writes() {
        for (key, value, expected_condition) in [
            ("sync_merge_pending_v1", "legacy-receipt", "legacy"),
            ("sync_merge_pending_v2", "{", "malformed"),
        ] {
            let temp = TempDir::new().unwrap();
            let obr_dir = temp.path().join(".beads");
            fs::create_dir_all(&obr_dir).unwrap();
            let db_path = obr_dir.join("beads.db");
            let jsonl_path = obr_dir.join("issues.jsonl");
            let mut storage = obr::storage::SqliteStorage::open(&db_path).unwrap();
            storage.set_metadata(key, value).unwrap();
            drop(storage);
            fs::write(&jsonl_path, b"{\"id\":\"br-existing\"}\n").unwrap();
            let database_before = fs::read(&db_path).unwrap();
            let jsonl_before = fs::read(&jsonl_path).unwrap();
            let authority = Arc::new(
                obr::sync::blocking_database_family_write_lock_with_timeout(
                    &obr_dir,
                    &db_path,
                    Some(1_000),
                )
                .unwrap(),
            );

            let state =
                inspect_pending_sync_merge_for_startup_under_authority(&db_path, &authority)
                    .unwrap()
                    .expect("pending state must refuse no-DB writer");
            let err = pending_sync_merge_no_db_refusal_error(&state);

            assert_eq!(state.condition_name(), expected_condition);
            assert!(
                err.to_string().contains("without `--no-db`"),
                "refusal must direct the caller to DB-backed merge recovery: {err}"
            );
            assert_eq!(
                fs::read(&db_path).unwrap(),
                database_before,
                "{expected_condition} inspection changed database bytes"
            );
            assert_eq!(
                fs::read(&jsonl_path).unwrap(),
                jsonl_before,
                "{expected_condition} inspection changed JSONL bytes"
            );
        }
    }

    #[test]
    fn caller_write_lock_scope_is_path_specific() {
        let mut overrides = build_cli_overrides(&Cli::parse_from([
            "obr",
            "--no-auto-import",
            "--no-auto-flush",
            "list",
        ]));
        let temp = TempDir::new().expect("tempdir");
        let obr_dir = temp.path().join("one").join(".beads");
        let other_dir = temp.path().join("two").join(".beads");
        std::fs::create_dir_all(&obr_dir).expect("create obr dir");
        let database_path = obr_dir.join("beads.db");
        let guard = Arc::new(
            obr::sync::blocking_database_family_write_lock_with_timeout(
                &obr_dir,
                &database_path,
                Some(0),
            )
            .expect("lock database family"),
        );
        overrides.mark_database_family_lock_held(&obr_dir, &guard);

        assert!(overrides.holds_write_lock_for(&obr_dir));
        assert!(!overrides.holds_write_lock_for(&other_dir));
    }

    #[test]
    fn help_includes_core_commands() {
        let help = Cli::command().render_help().to_string();
        assert!(help.contains("create"));
        assert!(help.contains("list"));
        assert!(help.contains("sync"));
        assert!(help.contains("ready"));
    }

    #[test]
    fn version_includes_name_and_version() {
        let version = Cli::command().render_version();
        assert!(version.contains("obr"));
        assert!(version.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn is_mutating_command_detects_mutations() {
        let create_cmd = Commands::Create(make_create_args());
        let list_cmd = Commands::List(obr::cli::ListArgs::default());
        assert!(is_mutating_command(&create_cmd));
        assert!(!is_mutating_command(&list_cmd));
    }

    #[test]
    fn is_mutating_command_distinguishes_read_only_subcommands() {
        let dep_list = Cli::parse_from(["obr", "dep", "list", "bd-123"]).command;
        let dep_add = Cli::parse_from(["obr", "dep", "add", "bd-123", "bd-456"]).command;
        let label_list = Cli::parse_from(["obr", "label", "list"]).command;
        let label_add =
            Cli::parse_from(["obr", "label", "add", "bd-123", "--label", "ops"]).command;
        let comments_list = Cli::parse_from(["obr", "comments", "bd-123"]).command;
        let comments_add = Cli::parse_from(["obr", "comments", "add", "bd-123", "hello"]).command;
        let orphans = Cli::parse_from(["obr", "orphans"]).command;
        let orphans_fix = Cli::parse_from(["obr", "orphans", "--fix"]).command;

        assert!(!is_mutating_command(&dep_list));
        assert!(is_mutating_command(&dep_add));
        assert!(!is_mutating_command(&label_list));
        assert!(is_mutating_command(&label_add));
        assert!(!is_mutating_command(&comments_list));
        assert!(is_mutating_command(&comments_add));
        assert!(!is_mutating_command(&orphans));
        assert!(is_mutating_command(&orphans_fix));
    }

    #[test]
    fn sync_is_not_auto_imported_or_auto_flushed() {
        let sync_cmd = Cli::parse_from(["obr", "sync"]).command;
        assert!(!is_mutating_command(&sync_cmd));
        assert!(!should_auto_import(&sync_cmd));
    }

    #[test]
    fn pending_merge_guard_allows_only_merge_resume_among_sync_mutations() {
        let merge = Cli::parse_from(["obr", "sync", "--merge"]).command;
        let force_db = Cli::parse_from(["obr", "sync", "--merge", "--force-db"]).command;
        let force_jsonl = Cli::parse_from(["obr", "sync", "--merge", "--force-jsonl"]).command;
        for command in [&merge, &force_db, &force_jsonl] {
            assert!(command_is_sync_merge(command));
            assert!(
                !command_must_refuse_during_pending_merge(command),
                "dedicated merge resume must remain available: {command:?}"
            );
            assert_eq!(
                pending_merge_startup_disposition(command),
                PendingMergeStartupDisposition::Resume
            );
        }

        let mutations = vec![
            Cli::parse_from(["obr", "sync", "--flush-only"]).command,
            Cli::parse_from(["obr", "sync", "--import-only"]).command,
            Cli::parse_from(["obr", "sync", "--reconcile"]).command,
            Cli::parse_from(["obr", "sync", "--reconcile-additive", "--apply"]).command,
        ];
        for command in &mutations {
            assert!(!command_is_sync_merge(command), "{command:?}");
            assert!(
                command_must_refuse_during_pending_merge(command),
                "non-merge sync mutation must fail closed: {command:?}"
            );
            assert_eq!(
                pending_merge_startup_disposition(command),
                PendingMergeStartupDisposition::Refuse
            );
        }
    }

    #[test]
    fn pending_merge_guard_covers_every_non_sync_mutation_family() {
        let mutations = vec![
            Cli::parse_from(["obr", "create", "fixture"]).command,
            Cli::parse_from(["obr", "update", "bd-one", "--title", "changed"]).command,
            Cli::parse_from(["obr", "delete", "bd-one"]).command,
            Cli::parse_from(["obr", "dep", "add", "bd-one", "bd-two"]).command,
            Cli::parse_from(["obr", "label", "add", "bd-one", "--label", "p0"]).command,
            Cli::parse_from([
                "obr",
                "gate",
                "report",
                "bd-one",
                "--gate",
                "ci",
                "--provider",
                "test",
                "--status",
                "pass",
            ])
            .command,
            Cli::parse_from([
                "obr",
                "capacity",
                "exempt",
                "bd-one",
                "--status",
                "blocked",
                "--provider",
                "operator",
                "--reason",
                "pending merge guard fixture",
            ])
            .command,
            Cli::parse_from([
                "obr",
                "capacity",
                "renew",
                "bd-one",
                "--status",
                "blocked",
                "--provider",
                "operator",
            ])
            .command,
            Cli::parse_from([
                "obr",
                "capacity",
                "revoke",
                "bd-one",
                "--status",
                "blocked",
                "--provider",
                "operator",
            ])
            .command,
            Cli::parse_from(["obr", "query", "save", "mine"]).command,
            Cli::parse_from(["obr", "config", "set", "sync.auto_flush=true"]).command,
            Cli::parse_from(["obr", "config", "edit"]).command,
            Cli::parse_from(["obr", "history", "restore", "fixture.jsonl", "--force"]).command,
            Cli::parse_from(["obr", "audit", "record", "--kind", "tool_call"]).command,
            Cli::parse_from(["obr", "doctor", "--repair"]).command,
            Cli::parse_from(["obr", "doctor", "--repair-indexes"]).command,
            Cli::parse_from(["obr", "doctor", "undo", "latest"]).command,
            Cli::parse_from(["obr", "agents", "--add", "--force"]).command,
        ];

        for command in &mutations {
            assert!(
                command_must_refuse_during_pending_merge(command),
                "pending receipt must refuse mutation before command execution: {command:?}"
            );
            assert_eq!(
                pending_merge_startup_disposition(command),
                PendingMergeStartupDisposition::Refuse
            );
        }
    }

    #[test]
    fn pending_merge_guard_preserves_read_only_commands() {
        let read_only = vec![
            Cli::parse_from(["obr", "list"]).command,
            Cli::parse_from(["obr", "show", "bd-one"]).command,
            Cli::parse_from(["obr", "sync", "--status"]).command,
            Cli::parse_from(["obr", "sync", "--witness"]).command,
            Cli::parse_from(["obr", "sync", "--reconcile", "--dry-run"]).command,
            Cli::parse_from(["obr", "sync", "--reconcile-additive"]).command,
            Cli::parse_from(["obr", "gate", "list", "bd-one"]).command,
            Cli::parse_from(["obr", "capacity", "exemptions", "bd-one"]).command,
            Cli::parse_from(["obr", "query", "list"]).command,
            Cli::parse_from(["obr", "config", "list"]).command,
            Cli::parse_from(["obr", "history", "list"]).command,
            Cli::parse_from(["obr", "audit", "summary"]).command,
            Cli::parse_from(["obr", "doctor", "--repair", "--dry-run"]).command,
            Cli::parse_from(["obr", "doctor", "undo", "latest", "--dry-run"]).command,
            Cli::parse_from(["obr", "agents", "--add", "--dry-run"]).command,
        ];

        for command in &read_only {
            assert!(
                !command_must_refuse_during_pending_merge(command),
                "read-only command must remain usable: {command:?}"
            );
            assert_eq!(
                pending_merge_startup_disposition(command),
                PendingMergeStartupDisposition::ReadOnlyNoAutoSync,
                "read-only command must disable automatic import/export while pending"
            );
        }
    }

    #[test]
    fn no_db_sync_write_intent_covers_jsonl_rewriters_only() {
        let flush = Cli::parse_from(["obr", "sync", "--flush-only"]).command;
        let merge = Cli::parse_from(["obr", "sync", "--merge"]).command;
        let force_merge = Cli::parse_from(["obr", "sync", "--merge", "--force-jsonl"]).command;
        let import = Cli::parse_from(["obr", "sync", "--import-only"]).command;
        let status = Cli::parse_from(["obr", "sync", "--status"]).command;
        let witness = Cli::parse_from(["obr", "sync", "--witness"]).command;
        let additive_plan = Cli::parse_from(["obr", "sync", "--reconcile-additive"]).command;

        assert!(no_db_jsonl_write_intent(&flush));
        assert!(no_db_jsonl_write_intent(&merge));
        assert!(no_db_jsonl_write_intent(&force_merge));
        assert!(!no_db_jsonl_write_intent(&import));
        assert!(!no_db_jsonl_write_intent(&status));
        assert!(!no_db_jsonl_write_intent(&witness));
        assert!(!no_db_jsonl_write_intent(&additive_plan));
    }

    #[test]
    fn sync_modes_require_write_lock_before_storage_open() {
        // Regression: `obr sync --flush-only` calls `finalize_export` inside a
        // `with_write_transaction` (clears dirty flags, updates
        // jsonl_content_hash + last_export_time + needs_flush metadata, writes
        // export hashes). That makes it a write-side operation as far as
        // fsqlite is concerned. Previously the `needs_write_lock` match arm
        // excluded `--flush-only`, leaving two concurrent `obr sync
        // --flush-only` invocations — or one racing a mutating command's
        // auto-flush — to hit the fsqlite concurrent-write deadlock that the
        // `.write.lock` was specifically introduced (issue #243) to prevent.
        //
        // `obr sync --status` is read-only after storage is open, but the open
        // path can apply runtime metadata defaults, recover from JSONL, or move
        // sidecars. It must therefore serialize before entering `sync::execute`.
        let flush_only = Cli::parse_from(["obr", "sync", "--flush-only"]).command;
        let status = Cli::parse_from(["obr", "sync", "--status"]).command;
        let witness = Cli::parse_from(["obr", "sync", "--witness"]).command;
        let merge = Cli::parse_from(["obr", "sync", "--merge"]).command;
        let import_only = Cli::parse_from(["obr", "sync", "--import-only"]).command;
        let reconcile_plan = Cli::parse_from(["obr", "sync", "--reconcile-additive"]).command;
        let reconcile_apply =
            Cli::parse_from(["obr", "sync", "--reconcile-additive", "--apply"]).command;
        let default_sync = Cli::parse_from(["obr", "sync"]).command;

        assert!(
            needs_write_lock(&flush_only),
            "`obr sync --flush-only` writes DB metadata and must serialize via .write.lock"
        );
        assert!(
            needs_write_lock(&status),
            "`obr sync --status` opens storage and must serialize before recovery/schema work"
        );
        assert!(
            !needs_write_lock(&witness),
            "`obr sync --witness` reads JSONL without opening SQLite and should not wait on .write.lock"
        );
        assert!(needs_write_lock(&merge));
        assert!(needs_write_lock(&import_only));
        assert!(
            !needs_write_lock(&reconcile_plan),
            "additive dry-run opens the current database read-only and must not acquire the writer lock"
        );
        assert!(
            !needs_write_lock(&reconcile_apply),
            "additive apply owns redirect-aware resolution and its exact terminal workspace lock inside the hardened reviewed-apply wrapper"
        );
        assert!(
            !needs_write_lock(&default_sync),
            "bare `obr sync` fails validation before storage open and should not wait on .write.lock"
        );
    }

    /// The doctor write lock follows MUTATION, not subcommand shape.
    ///
    /// `obr-m6m`: this used to assert that a bare `obr doctor` takes the lock,
    /// on the reasoning that it "copies/probes the live DB family". It does
    /// neither under that lock — `is_mutating_command` and `should_auto_import`
    /// are both false for Doctor, so `should_preopen_storage` is false and no
    /// storage is opened while it is held. What the lock did accomplish was to
    /// blind doctor's own `write_lock` check, which probes for an orphaned
    /// `.write.lock` by trying to take it: holding it here made the probe
    /// collide with its caller and answer `probe_would_block_live_holder` on
    /// every workspace, including ones with no holder at all. The flags
    /// `--repair` / `--fix` / `--repair-indexes` live on the same `None`
    /// subcommand, which is why one arm could not tell the two apart.
    #[test]
    fn doctor_write_lock_follows_mutation_not_subcommand_shape() {
        let inspect = Cli::parse_from(["obr", "doctor"]).command;
        let inspect_json = Cli::parse_from(["obr", "doctor", "--json"]).command;
        let repair = Cli::parse_from(["obr", "doctor", "--repair"]).command;
        let fix = Cli::parse_from(["obr", "doctor", "--fix"]).command;
        let repair_indexes = Cli::parse_from(["obr", "doctor", "--repair-indexes"]).command;
        let capabilities =
            Cli::parse_from(["obr", "doctor", "capabilities", "--format", "json"]).command;
        let robot_docs =
            Cli::parse_from(["obr", "doctor", "robot-docs", "--format", "json"]).command;
        let health = Cli::parse_from(["obr", "doctor", "health", "--json"]).command;
        let ls = Cli::parse_from(["obr", "doctor", "ls", "--json"]).command;
        let explain =
            Cli::parse_from(["obr", "doctor", "explain", "permissions.write_lock"]).command;
        let undo = Cli::parse_from(["obr", "doctor", "undo", "latest"]).command;

        assert!(
            !needs_write_lock(&inspect),
            "a read-only `obr doctor` must not take the lock its own write_lock probe tests"
        );
        assert!(
            !needs_write_lock(&inspect_json),
            "--json changes the rendering, not the mutation profile"
        );
        assert!(
            needs_write_lock(&repair),
            "`--repair` mutates and must hold write authority"
        );
        assert!(
            needs_write_lock(&fix),
            "`--fix` is a visible alias of `--repair` and must not slip past the lock decision"
        );
        assert!(needs_write_lock(&repair_indexes));
        assert!(needs_write_lock(&undo));
        assert!(
            !needs_write_lock(&capabilities),
            "capabilities is a pure contract surface and must not depend on workspace lock health"
        );
        assert!(
            !needs_write_lock(&robot_docs),
            "robot-docs is a pure help surface and must not depend on workspace lock health"
        );
        assert!(
            !needs_write_lock(&health),
            "health is read-only filesystem liveness and must not acquire the DB write lock"
        );
        assert!(
            !needs_write_lock(&ls),
            "ls only reads .doctor/runs and must not acquire the DB write lock"
        );
        assert!(
            !needs_write_lock(&explain),
            "explain is a read-only diagnostic surface and must not acquire the DB write lock"
        );
    }

    #[test]
    fn diagnostic_and_config_commands_skip_auto_import() {
        let cases: &[&[&str]] = &[
            &["obr", "doctor"],
            &["obr", "capabilities"],
            &["obr", "robot-docs", "guide"],
            &["obr", "where"],
            &["obr", "schema"],
            &["obr", "config", "path"],
            &["obr", "history", "list"],
            &["obr", "orphans"],
        ];

        for argv in cases {
            let command = Cli::parse_from(*argv).command;
            assert!(
                !should_auto_import(&command),
                "command should not auto-import: {command:?}"
            );
        }
    }

    #[test]
    fn orphans_defers_auto_import_but_keeps_write_lock_when_initialized() {
        let command = Cli::parse_from(["obr", "orphans"]).command;
        assert!(!should_auto_import(&command));
        assert!(needs_write_lock(&command));
    }

    #[test]
    fn orphans_fix_uses_mutating_flush_pipeline_without_startup_auto_import() {
        let command = Cli::parse_from(["obr", "orphans", "--fix"]).command;
        assert!(is_mutating_command(&command));
        assert!(!should_auto_import(&command));
        assert!(needs_write_lock(&command));
    }

    #[test]
    fn direct_storage_inspection_commands_require_write_lock() {
        let cases: &[&[&str]] = &[
            &["obr", "list"],
            &["obr", "audit", "summary"],
            &["obr", "config", "list"],
            &["obr", "info"],
            &["obr", "where"],
        ];

        for argv in cases {
            let command = Cli::parse_from(*argv).command;
            assert!(
                needs_write_lock(&command),
                "storage-opening command should serialize DB-family access: {command:?}"
            );
        }
    }

    #[test]
    fn config_path_and_edit_do_not_require_db_write_lock() {
        let cases: &[&[&str]] = &[
            &["obr", "config", "path"],
            &["obr", "config", "edit"],
            &["obr", "capabilities"],
            &["obr", "robot-docs", "guide"],
        ];

        for argv in cases {
            let command = Cli::parse_from(*argv).command;
            assert!(
                !needs_write_lock(&command),
                "config command should not lock when it does not inspect the DB: {command:?}"
            );
        }
    }

    #[test]
    fn auto_import_expected_prefix_uses_merged_config_layers() {
        let temp = TempDir::new().expect("tempdir");
        let obr_dir = temp.path().join(".beads");
        fs::create_dir_all(&obr_dir).expect("create obr dir");
        fs::write(
            obr_dir.join("config.yaml"),
            "issue_prefix: document-intelligence\n",
        )
        .expect("write config");

        let mut storage_result =
            config::open_storage_with_cli(&obr_dir, &config::CliOverrides::default())
                .expect("open storage");
        storage_result
            .storage
            .set_config("issue_prefix", "db-prefix")
            .expect("set db prefix");

        let prefix =
            resolve_auto_import_expected_prefix(&storage_result, &config::CliOverrides::default())
                .expect("resolve prefix");

        assert_eq!(prefix, "document-intelligence");
    }

    #[test]
    fn preopened_storage_reuses_startup_paths() {
        let temp = TempDir::new().expect("tempdir");
        let obr_dir = temp.path().join(".beads");
        fs::create_dir_all(&obr_dir).expect("create obr dir");

        let first_jsonl = obr_dir.join("first.jsonl");
        let second_jsonl = obr_dir.join("second.jsonl");
        let metadata_path = obr_dir.join("metadata.json");
        fs::write(
            &metadata_path,
            r#"{"database":"beads.db","jsonl_export":"first.jsonl"}"#,
        )
        .expect("write initial metadata");

        let mut overrides = config::CliOverrides::default();
        overrides.db = Some(obr_dir.join("beads.db"));
        overrides.no_db = Some(true);
        let mut ctx = StartupContext::init(&overrides).expect("startup context");

        fs::write(
            &metadata_path,
            r#"{"database":"beads.db","jsonl_export":"second.jsonl"}"#,
        )
        .expect("rewrite metadata");

        let storage_ctx = open_storage_from_ctx(&mut ctx, None).expect("preopened storage");

        assert_eq!(storage_ctx.paths.jsonl_path, first_jsonl);
        assert_ne!(storage_ctx.paths.jsonl_path, second_jsonl);
    }

    #[test]
    fn create_dispatch_reuses_preopened_storage_context() {
        let temp = TempDir::new().expect("tempdir");
        let obr_dir = temp.path().join(".beads");
        fs::create_dir_all(&obr_dir).expect("create obr dir");

        let first_db = obr_dir.join("first.db");
        let second_db = obr_dir.join("second.db");
        let metadata_path = obr_dir.join("metadata.json");
        fs::write(
            &metadata_path,
            format!(
                r#"{{"database":"{}","jsonl_export":"issues.jsonl"}}"#,
                first_db.display()
            ),
        )
        .expect("write initial metadata");

        let overrides = config::CliOverrides::default();
        let startup =
            config::load_startup_config_with_paths(&obr_dir, None).expect("startup context");

        fs::write(
            &metadata_path,
            format!(
                r#"{{"database":"{}","jsonl_export":"issues.jsonl"}}"#,
                second_db.display()
            ),
        )
        .expect("rewrite metadata");

        let cli = Cli::parse_from(["obr", "--json", "create", "Use preopened storage"]);
        let output_ctx = OutputContext::from_args(&cli);
        let Commands::Create(args) = cli.command else {
            unreachable!("expected create command");
        };
        let mut storage_result = Some(
            config::open_storage_with_startup_config(startup, &overrides, false)
                .expect("preopened storage"),
        );

        execute_create_command(&args, &overrides, &output_ctx, &mut storage_result)
            .expect("create should use preopened storage");

        assert!(storage_result.is_none());

        let first_storage = obr::storage::SqliteStorage::open(&first_db).expect("open first db");
        assert_eq!(first_storage.count_issues().expect("count first db"), 1);
        assert!(
            !second_db.exists(),
            "create dispatch reopened storage from rewritten metadata instead of using preopened context"
        );
    }

    #[test]
    fn should_render_errors_as_json_when_command_requests_json_format() {
        let cli = Cli::parse_from(["obr", "list", "--format", "json"]);
        assert!(should_render_errors_as_json_with_env(&cli, None));
    }

    #[test]
    fn should_render_errors_as_json_for_query_run_json_format() {
        let cli = Cli::parse_from(["obr", "query", "run", "saved", "--format", "json"]);
        assert!(should_render_errors_as_json_with_env(&cli, None));
    }

    #[test]
    fn should_render_errors_as_json_for_doctor_robot_triage() {
        let cli = Cli::parse_from(["obr", "doctor", "--robot-triage"]);
        assert!(should_render_errors_as_json_with_env(&cli, None));
    }

    #[test]
    fn restores_default_sigpipe_for_text_output_commands() {
        for argv in [
            vec!["br", "list"],
            vec!["br", "ready"],
            vec!["br", "lint"],
            vec!["br", "list", "--format", "csv"],
            vec!["br", "update", "bd-one", "--priority", "2"],
            vec!["br", "--quiet", "list"],
        ] {
            let cli = Cli::parse_from(argv.clone());
            let structured = should_render_errors_as_json_with_env(&cli, None);
            assert!(
                !structured,
                "{argv:?} is a text-output command and must not be classified as structured"
            );
            assert!(
                should_restore_default_sigpipe(&cli, structured),
                "{argv:?} must die quietly by SIGPIPE like any Unix filter (#434)"
            );
        }
    }

    #[test]
    fn keeps_sigpipe_ignored_for_structured_output_commands() {
        for (argv, env_format) in [
            (vec!["br", "list", "--json"], None),
            (vec!["br", "--json", "list"], None),
            (vec!["br", "list", "--format", "json"], None),
            (vec!["br", "list", "--format", "toon"], None),
            (vec!["br", "ready", "--robot"], None),
            (vec!["br", "doctor", "--robot-triage"], None),
            (vec!["br", "list"], Some(OutputFormat::Json)),
            (vec!["br", "list"], Some(OutputFormat::Toon)),
        ] {
            let cli = Cli::parse_from(argv.clone());
            let structured = should_render_errors_as_json_with_env(&cli, env_format);
            assert!(
                structured,
                "{argv:?} with env {env_format:?} must be classified as structured output"
            );
            assert!(
                !should_restore_default_sigpipe(&cli, structured),
                "{argv:?} streams JSON/TOON and must keep the exit-0 broken-pipe contract"
            );
        }
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn keeps_sigpipe_ignored_for_the_mcp_server() {
        let cli = Cli::parse_from(["br", "serve"]);
        let structured = should_render_errors_as_json_with_env(&cli, None);
        assert!(
            !should_restore_default_sigpipe(&cli, structured),
            "br serve must see EPIPE as an error so the stdio transport can shut down cooperatively"
        );
    }

    #[test]
    fn should_render_errors_as_json_when_command_requests_toon_format() {
        let cli = Cli::parse_from(["obr", "list", "--format", "toon"]);
        assert!(should_render_errors_as_json_with_env(&cli, None));
    }

    #[test]
    fn should_render_errors_as_json_when_env_requests_json_format() {
        let cli = Cli::parse_from(["obr", "history", "list"]);
        assert!(should_render_errors_as_json_with_env(
            &cli,
            Some(OutputFormat::Json)
        ));
    }

    #[test]
    fn should_render_errors_as_json_when_env_requests_toon_format() {
        let cli = Cli::parse_from(["obr", "history", "list"]);
        assert!(should_render_errors_as_json_with_env(
            &cli,
            Some(OutputFormat::Toon)
        ));
    }

    #[test]
    fn should_not_render_errors_as_json_without_json_request() {
        let cli = Cli::parse_from(["obr", "history", "list"]);
        assert!(!should_render_errors_as_json_with_env(&cli, None));
    }

    #[test]
    fn preopen_storage_skips_commands_without_bootstrap_or_flush_work() {
        assert!(!should_preopen_storage(true, false));
    }

    #[test]
    fn preopen_storage_keeps_mutating_auto_flush_path() {
        assert!(should_preopen_storage(true, true));
    }

    #[test]
    fn preopen_storage_keeps_bootstrap_path_for_staleness_checks() {
        assert!(should_preopen_storage(true, true));
    }

    #[test]
    fn preopen_storage_requires_write_lock_before_open() {
        assert!(should_acquire_startup_write_lock(false, true, false));
        assert!(should_acquire_startup_write_lock(true, false, false));
        assert!(should_acquire_startup_write_lock(true, true, false));
        assert!(!should_acquire_startup_write_lock(false, false, false));
        assert!(!should_acquire_startup_write_lock(false, true, true));
        assert!(!should_acquire_startup_write_lock(true, false, true));
        assert!(!should_acquire_startup_write_lock(true, true, true));
    }
}
