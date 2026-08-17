//! Legacy-name compatibility shims for the `beads`/`bd`/`br` → `obr` rename.
//!
//! Scope is deliberately narrow: only on-disk WORKSPACE artifacts that a
//! fork-era repository may still hold are read under their old names — the
//! `.beads`/`_beads` directories, `beads.db`, the in-dir `beads.jsonl` /
//! `issues.jsonl` exports, merge/history/recovery/snapshot filenames, the Org
//! drawer's `BEADS_SCHEMA_VERSION`, `br.*.v1` schema ids, and the agents
//! marker. Environment variables and user-config locations are NOT compatible:
//! `OBR_*` and `~/.config/obr/config.yaml` are the only spellings obr reads.
//! Reading a legacy artifact emits exactly one deprecation warning per
//! distinct legacy name per process.
//!
//! Warnings follow the `report_auto_flush_failure` house style
//! (`crate::cli::commands::report_auto_flush_failure`): stderr only, never
//! stdout, suppressed under `--quiet`, and emitted as a single-line
//! `{"warning":{…}}` envelope under `--json`/`--toon` so machine consumers can
//! keep parsing stdout.
//!
//! Every legacy read is either routed through this module or tagged with a
//! `legacy_compat:` marker comment at its site, so
//! `git grep -n 'legacy_compat' src/` enumerates the whole compatibility
//! surface for review and eventual removal. Keep new legacy reads to that
//! rule or the enumeration stops being true.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

/// Warning code carried by every deprecation envelope.
pub const DEPRECATED_NAME_CODE: &str = "DEPRECATED_NAME";
const ADVISORY_CODE: &str = "WORKSPACE_ADVISORY";

const MODE_TEXT: u8 = 0;
const MODE_JSON: u8 = 1;
const MODE_QUIET: u8 = 2;

/// How deprecation warnings should be rendered.
///
/// Deprecation warnings fire during config discovery and environment lookups,
/// long before an [`OutputContext`](crate::output::OutputContext) exists, so
/// the effective output mode is recorded process-wide instead of threaded
/// through every path-resolution signature.
static WARNING_MODE: AtomicU8 = AtomicU8::new(MODE_TEXT);

/// Record how deprecation warnings should be rendered for this process.
///
/// Called once from `main` after the raw argument vector has been inspected,
/// and before `Cli::parse`, because workspace discovery can emit a
/// deprecation warning before clap has produced any flags. `quiet` wins over
/// `json`: a quiet run emits nothing at all.
pub fn set_warning_output_mode(json: bool, quiet: bool) {
    let mode = if quiet {
        MODE_QUIET
    } else if json {
        MODE_JSON
    } else {
        MODE_TEXT
    };
    WARNING_MODE.store(mode, Ordering::Relaxed);
}

/// Infer the warning output mode from a raw argument vector.
///
/// Mirrors the clap flags that select machine or silent output. Kept flag-only
/// (no value parsing) so it cannot be confused by an option *argument* that
/// happens to spell `--json`.
pub fn set_warning_output_mode_from_args<I, S>(args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut json = false;
    let mut quiet = false;
    for arg in args {
        match arg.as_ref() {
            "--json" | "--robot" | "--toon" => json = true,
            "--quiet" | "-q" => quiet = true,
            _ => {}
        }
    }
    set_warning_output_mode(json, quiet);
}

/// Returns `true` the first time `key` is seen, `false` on every later call.
fn claim_first_use(key: &str) -> bool {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .is_ok_and(|mut seen| seen.insert(key.to_string()))
}

/// Emit a one-shot deprecation warning for `legacy`, pointing at `replacement`.
///
/// `detail` is the human sentence; it is sanitized for terminal output and
/// carried verbatim in the JSON envelope. Repeat calls for the same `legacy`
/// name are silently dropped, so callers may invoke this unconditionally from
/// hot lookup paths.
pub fn warn_deprecated_name(legacy: &str, replacement: &str, detail: &str) {
    tracing::warn!(legacy, replacement, "deprecated name in use");

    if WARNING_MODE.load(Ordering::Relaxed) == MODE_QUIET {
        return;
    }
    if !claim_first_use(legacy) {
        return;
    }

    if WARNING_MODE.load(Ordering::Relaxed) == MODE_JSON {
        let payload = serde_json::json!({
            "warning": {
                "code": DEPRECATED_NAME_CODE,
                "message": detail,
                "legacy": legacy,
                "replacement": replacement,
            }
        });
        eprintln!(
            "{}",
            serde_json::to_string(&payload).unwrap_or_else(|_| format!(
                "{{\"warning\":{{\"code\":\"{DEPRECATED_NAME_CODE}\"}}}}"
            ))
        );
        return;
    }

    eprintln!(
        "{}",
        crate::format::sanitize_terminal_text(&format!("Warning: {detail}"))
    );
}

/// Emit a one-shot advisory that is not a deprecation.
///
/// Same delivery rules as [`warn_deprecated_name`] — stderr only, suppressed
/// under `--quiet`, carried in the JSON envelope under `--json`/`--toon`, and
/// dropped after the first call for a given `key`.
pub fn warn_once(key: &str, detail: &str) {
    warn_once_with_code(ADVISORY_CODE, key, detail);
}

/// [`warn_once`] with a caller-chosen warning `code` in the JSON envelope.
///
/// The delivery rules are the reason this lives here rather than at each call
/// site — mode handling, the once-per-`key` claim, and terminal sanitization
/// are the same for every warning obr emits, whether or not it concerns a
/// legacy name. Machine consumers switch on `code`.
pub fn warn_once_with_code(code: &str, key: &str, detail: &str) {
    tracing::warn!(key, code, "advisory");

    if WARNING_MODE.load(Ordering::Relaxed) == MODE_QUIET {
        return;
    }
    if !claim_first_use(key) {
        return;
    }

    if WARNING_MODE.load(Ordering::Relaxed) == MODE_JSON {
        let payload = serde_json::json!({
            "warning": {
                "code": code,
                "message": detail,
                "key": key,
            }
        });
        eprintln!(
            "{}",
            serde_json::to_string(&payload)
                .unwrap_or_else(|_| format!("{{\"warning\":{{\"code\":\"{code}\"}}}}"))
        );
        return;
    }

    eprintln!(
        "{}",
        crate::format::sanitize_terminal_text(&format!("Warning: {detail}"))
    );
}

/// Warn that a deprecated on-disk artifact was read.
pub fn warn_deprecated_artifact(legacy: &str, replacement: &str, path: &Path) {
    warn_deprecated_name(
        legacy,
        replacement,
        &format!(
            "{legacy} is a legacy name and support for it will be removed; \
             rename it to {replacement} ({}).",
            path.display()
        ),
    );
}

/// Accept a schema identifier read back from an on-disk artifact.
///
/// Matches the current `obr.…` identifier, or the `br.…` spelling the same
/// artifact would have carried before the rename. Only identifiers that are
/// *read* need this: write-only schema IDs were renamed outright.
#[must_use]
pub fn schema_id_accepted(found: &str, current: &str) -> bool {
    if found == current {
        return true;
    }
    let Some(suffix) = current.strip_prefix("obr.") else {
        return false;
    };
    if found.strip_prefix("br.") != Some(suffix) {
        return false;
    }
    warn_deprecated_name(
        found,
        current,
        &format!(
            "artifact declares the pre-rename schema {found}; it is still \
             accepted, but new artifacts are written as {current}."
        ),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_use_is_claimed_once() {
        let key = "legacy-compat-test-unique-key";
        assert!(claim_first_use(key));
        assert!(!claim_first_use(key));
    }

    #[test]
    fn args_scan_detects_machine_and_quiet_modes() {
        set_warning_output_mode_from_args(["br", "list", "--json"]);
        assert_eq!(WARNING_MODE.load(Ordering::Relaxed), MODE_JSON);
        set_warning_output_mode_from_args(["br", "list", "--json", "--quiet"]);
        assert_eq!(WARNING_MODE.load(Ordering::Relaxed), MODE_QUIET);
        set_warning_output_mode_from_args(["br", "list"]);
        assert_eq!(WARNING_MODE.load(Ordering::Relaxed), MODE_TEXT);
    }
}
