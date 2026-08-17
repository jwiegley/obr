//! Shared pseudo-terminal plumbing for the Rich-mode integration tests.
//!
//! `obr` selects Rich output only when stdout is a terminal, and every other
//! test helper in this estate forces `NO_COLOR=1` or a pipe — so the Rich
//! branch of 33 files is only reachable from here. This module exists so
//! there is exactly ONE copy of the pty setup: the previous single copy lived
//! inside `golden_rich_panels.rs` and had been dead on macOS for four
//! releases without anyone noticing, which is precisely what a second copy
//! would invite.
//!
//! Included with `#[path]` rather than through `tests/common/mod.rs`, so a
//! test that needs a terminal does not drag in the whole shared harness.
#![allow(dead_code)]

use assert_cmd::Command;
use std::ffi::OsStr;
use std::path::Path;

/// The comment author obr records is the invoking user (`USER`/`LOGNAME`/
/// `USERNAME`), so it lands verbatim in any captured output. Left to the host
/// it makes a snapshot pass only for whoever recorded it; pinning it here is
/// what makes these tests portable. The value is the one already frozen in
/// the committed snapshots.
pub const FIXTURE_USER: &str = "ubuntu";

/// Path to the `obr` binary built by the current cargo invocation.
pub fn obr_bin() -> &'static Path {
    assert_cmd::cargo::cargo_bin!("obr")
}

/// Environment variables that must never leak from the developer's shell.
///
/// `OBR_DIR`/`OBR_CACHE_DIR` relocate the workspace and the database, which
/// is precisely what these tests inspect — an inherited one would make the
/// assertions describe the developer's shell, not the fixture. The legacy
/// `BD_`/`BEADS_` prefixes are still honoured by the config layer, so they
/// have to go too.
pub fn should_clear_inherited_obr_env(key: &OsStr) -> bool {
    let key = key.to_string_lossy();
    key.starts_with("BD_")
        || key.starts_with("BEADS_")
        || key.starts_with("BR_")
        || key.starts_with("OBR_")
        || matches!(
            key.as_ref(),
            "TOON_DEFAULT_FORMAT" | "TOON_STATS" | "NO_COLOR"
        )
}

pub fn clear_inherited_obr_env(cmd: &mut Command) {
    for (key, _) in std::env::vars_os() {
        if should_clear_inherited_obr_env(&key) {
            cmd.env_remove(key);
        }
    }
}

pub fn pin_fixture_identity(cmd: &mut Command) {
    for key in ["USER", "LOGNAME", "USERNAME"] {
        cmd.env(key, FIXTURE_USER);
    }
}

fn sh_quote(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// `script(1)` arguments that run `command_line` under a pseudo-terminal.
///
/// The two implementations are not compatible: util-linux takes the command
/// via `-c`, BSD/macOS takes it as trailing arguments after the typescript
/// file and rejects `-c` outright. The Linux-only form used to be hardcoded,
/// so every Rich-mode golden failed on macOS with `script: illegal option --
/// c` — which is a large part of why TTY-only output strings could rot
/// unnoticed.
fn script_args(command_line: &str) -> Vec<String> {
    let sh = ["/bin/sh".to_string(), "-c".to_string(), command_line.into()];
    if cfg!(target_os = "macos") {
        std::iter::once("-q".to_string())
            .chain(std::iter::once("/dev/null".to_string()))
            .chain(sh)
            .collect()
    } else {
        vec![
            "-q".to_string(),
            "-e".to_string(),
            "-c".to_string(),
            command_line.to_string(),
            "/dev/null".to_string(),
        ]
    }
}

/// One `obr` run on a real pseudo-terminal.
pub struct PtyRun {
    pub success: bool,
    /// Everything the terminal received (stdout and stderr are the same
    /// device here), with pty and ANSI noise removed but content untouched.
    pub text: String,
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            output.push(ch);
            continue;
        }

        if chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if ('@'..='~').contains(&code) {
                    break;
                }
            }
        }
    }

    output
}

/// Remove everything the pty added and nothing the program wrote.
///
/// BSD `script` feeds the pty an EOF and the line discipline echoes it back —
/// as the literal two characters `^D` (ECHOCTL), followed by backspaces —
/// ahead of any program output. util-linux does not. None of it is content,
/// and it only ever appears at the very start.
pub fn pty_text(raw: &str) -> String {
    let without_backspaces = raw.replace('\u{8}', "");
    let without_pty_preamble = without_backspaces
        .strip_prefix("^D")
        .unwrap_or(&without_backspaces);
    let normalized_newlines = without_pty_preamble
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let without_script_markers = normalized_newlines
        .lines()
        .filter(|line| !line.starts_with("Script started") && !line.starts_with("Script done"))
        .collect::<Vec<_>>()
        .join("\n");
    strip_ansi(&without_script_markers)
}

/// Run `obr <args>` under a pseudo-terminal `width` columns wide.
///
/// Returns whatever the terminal received, whether or not obr succeeded —
/// callers that need the exit status get it in [`PtyRun::success`].
pub fn rich_obr(root: &Path, width: usize, args: &[&str]) -> PtyRun {
    let mut command_parts = vec![sh_quote(obr_bin().as_os_str())];
    command_parts.extend(args.iter().map(|arg| sh_quote(OsStr::new(arg))));
    // `stty` and not just COLUMNS: a pty created by a process with no
    // controlling terminal starts at 0x0, and the renderer asks the terminal
    // first — a 0-width panel is what comes back otherwise.
    let command_line = format!(
        "stty columns {width} rows 40; COLUMNS={width} exec {}",
        command_parts.join(" ")
    );

    let mut cmd = Command::new("script");
    cmd.current_dir(root);
    cmd.args(script_args(&command_line));
    clear_inherited_obr_env(&mut cmd);
    cmd.env("HOME", root);
    cmd.env("COLUMNS", width.to_string());
    cmd.env("RUST_LOG", "error");
    cmd.env("RUST_BACKTRACE", "1");
    pin_fixture_identity(&mut cmd);

    let output = cmd.output().expect("run obr under pseudo-terminal");
    PtyRun {
        success: output.status.success(),
        text: pty_text(&String::from_utf8_lossy(&output.stdout)),
    }
}

/// [`rich_obr`], asserting the run succeeded.
pub fn rich_obr_ok(root: &Path, width: usize, args: &[&str]) -> String {
    let run = rich_obr(root, width, args);
    assert!(
        run.success,
        "rich obr command failed at width {width}: {args:?}\noutput:\n{}",
        run.text
    );
    run.text
}
