//! Logging configuration and initialization.
//!
//! Uses tracing with environment-based filtering and optional JSON file output.

use std::io::IsTerminal;
use std::path::Path;
use std::sync::{Mutex, Once};

use crate::error::{BeadsError, Result};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

/// Initialize logging for the CLI.
///
/// Logging honors `RUST_LOG` if set; otherwise a default filter is used based
/// on verbosity and quiet flags.
///
/// # Errors
///
/// Returns an error if logging initialization fails.
pub fn init_logging(verbosity: u8, quiet: bool, log_file: Option<&Path>) -> Result<()> {
    let env_filter = resolve_env_filter(verbosity, quiet)?;

    let fmt_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_level(true)
        .with_file(cfg!(debug_assertions))
        .with_line_number(cfg!(debug_assertions))
        .with_ansi(std::io::stderr().is_terminal());

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer);

    if let Some(path) = log_file {
        let file = std::fs::File::create(path)?;
        let file_layer = fmt::layer()
            .with_writer(Mutex::new(file))
            .with_ansi(false)
            .json();
        tracing::subscriber::set_global_default(subscriber.with(file_layer)).map_err(|err| {
            BeadsError::internal(format!("failed to initialize tracing subscriber: {err}"))
        })?;
    } else {
        tracing::subscriber::set_global_default(subscriber).map_err(|err| {
            BeadsError::internal(format!("failed to initialize tracing subscriber: {err}"))
        })?;
    }

    Ok(())
}

fn resolve_env_filter(verbosity: u8, quiet: bool) -> Result<EnvFilter> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_filter(verbosity, quiet)))
        .map_err(|err| BeadsError::Config(format!("failed to build log filter: {err}")))?;
    Ok(filter)
}

#[cfg(test)]
fn resolve_env_filter_with_override(
    verbosity: u8,
    quiet: bool,
    env_override: Option<&str>,
) -> Result<EnvFilter> {
    if let Some(value) = env_override {
        let filter = EnvFilter::try_new(value)
            .or_else(|_| EnvFilter::try_new(default_filter(verbosity, quiet)))
            .map_err(|err| BeadsError::Config(format!("failed to build log filter: {err}")))?;
        return Ok(filter);
    }
    resolve_env_filter(verbosity, quiet)
}

fn default_filter(verbosity: u8, quiet: bool) -> String {
    if quiet {
        return "error".to_string();
    }

    // fsqlite's internal submodules (btree cells, VDBE steps, cx checkpoints,
    // pager I/O) fire at `debug` for every row and page touched — enabling
    // them unfiltered drowns out obr's own logs by many orders of
    // magnitude on bulk imports. Keep fsqlite at `error` for default debug
    // builds; `-v` raises it to `warn`, and `-vv`/higher opt into more detail.
    match verbosity {
        0 => {
            if cfg!(debug_assertions) {
                "obr=debug,fsqlite=error".to_string()
            } else {
                "error".to_string()
            }
        }
        1 => "obr=debug,fsqlite=warn".to_string(),
        2 => "obr=debug,fsqlite=info,fsqlite_btree=warn,fsqlite_vdbe=warn,fsqlite_pager=warn"
            .to_string(),
        _ => "obr=trace,fsqlite=debug,fsqlite_btree=info,fsqlite_vdbe=info".to_string(),
    }
}

/// Initialize logging for tests with the test writer.
pub fn init_test_logging() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter("obr=debug,test=debug")
            .with_test_writer()
            .try_init()
            .ok();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static INIT_LOGGING: Once = Once::new();

    #[test]
    fn default_filter_respects_quiet() {
        assert_eq!(default_filter(0, true), "error");
    }

    #[test]
    fn default_filter_varies_with_verbosity() {
        assert_eq!(default_filter(1, false), "obr=debug,fsqlite=warn");
        // Verbosity 0 is the one level whose default depends on the build: a
        // debug build talks about itself, a release build stays quiet. Asserting
        // only the debug string made this test fail under `--release` for a
        // reason that had nothing to do with logging.
        assert_eq!(
            default_filter(0, false),
            if cfg!(debug_assertions) {
                "obr=debug,fsqlite=error"
            } else {
                "error"
            }
        );
        assert_eq!(
            default_filter(2, false),
            "obr=debug,fsqlite=info,fsqlite_btree=warn,fsqlite_vdbe=warn,fsqlite_pager=warn"
        );
        assert_eq!(
            default_filter(3, false),
            "obr=trace,fsqlite=debug,fsqlite_btree=info,fsqlite_vdbe=info"
        );
    }

    #[test]
    fn resolve_env_filter_prefers_rust_log() {
        let filter = resolve_env_filter_with_override(0, false, Some("obr=trace")).expect("filter");
        let rendered = filter.to_string();
        assert!(
            rendered.contains("obr=trace"),
            "expected env override to include trace, got {rendered}"
        );
    }

    #[test]
    fn resolve_env_filter_falls_back_on_invalid_env() {
        // Use a string that definitely fails to parse (unbalanced brackets)
        let filter =
            resolve_env_filter_with_override(1, false, Some("[invalid")).expect("fallback filter");
        let rendered = filter.to_string();
        assert!(
            rendered.contains("obr=debug"),
            "expected fallback filter, got {rendered}"
        );
    }

    #[test]
    fn init_test_logging_is_idempotent() {
        init_test_logging();
        init_test_logging();
    }

    #[test]
    fn init_logging_does_not_panic() {
        let result = std::panic::catch_unwind(|| {
            INIT_LOGGING.call_once(|| {
                let temp = tempfile::NamedTempFile::new().expect("temp log file");
                let result = init_logging(0, false, Some(temp.path()));
                if let Err(err) = result {
                    let message = err.to_string();
                    let is_already_set = message.contains("global")
                        || message.contains("already")
                        || message.contains("set");
                    assert!(is_already_set, "unexpected init_logging error: {message}");
                }
            });
        });
        assert!(result.is_ok());
    }
}
