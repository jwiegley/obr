//! Shell completions generation command.
//!
//! Generates shell completion scripts for bash, zsh, fish, `PowerShell`, and elvish.
//!
//! # Usage
//!
//! ```bash
//! # Generate bash completions to stdout
//! br completions bash
//!
//! # Generate zsh completions to a file
//! br completions zsh -o ~/.zsh/completions/_br
//!
//! # Generate fish completions
//! br completions fish > ~/.config/fish/completions/br.fish
//! ```

use crate::cli::{Cli, CompletionsArgs, ShellType};
use crate::error::Result;
use crate::output::OutputContext;
use clap::CommandFactory;
use clap_complete::env::Shells;
use std::io;
use tracing::info;

/// Execute the completions command.
///
/// # Errors
///
/// Returns an error if file I/O fails.
pub fn execute(args: &CompletionsArgs, _ctx: &OutputContext) -> Result<()> {
    info!(shell = ?args.shell, output = ?args.output, "Generating shell completions");

    let cmd = Cli::command();

    if let Some(output_path) = &args.output {
        // Generate to file
        let mut file = std::fs::File::create(output_path)?;
        write_dynamic_completions(args.shell, &cmd, &mut file)?;
        info!(path = %output_path.display(), "Wrote completion script");
        eprintln!(
            "Generated {} completions to {}",
            shell_name(args.shell),
            output_path.display()
        );
    } else {
        // Generate to stdout
        write_dynamic_completions(args.shell, &cmd, &mut io::stdout())?;
    }

    Ok(())
}

fn write_dynamic_completions(
    shell: ShellType,
    cmd: &clap::Command,
    out: &mut dyn io::Write,
) -> io::Result<()> {
    let shells = Shells::builtins();
    let Some(env_shell) = shells.completer(shell_env_name(shell)) else {
        return Err(std::io::Error::other(format!(
            "Unsupported shell: {}",
            shell_name(shell)
        )));
    };

    let bin = cmd.get_bin_name().unwrap_or_else(|| cmd.get_name());
    env_shell.write_registration("COMPLETE", cmd.get_name(), bin, bin, out)?;
    Ok(())
}

/// Get human-readable shell name.
const fn shell_name(shell: ShellType) -> &'static str {
    match shell {
        ShellType::Bash => "bash",
        ShellType::Zsh => "zsh",
        ShellType::Fish => "fish",
        ShellType::PowerShell => "PowerShell",
        ShellType::Elvish => "elvish",
    }
}

const fn shell_env_name(shell: ShellType) -> &'static str {
    match shell {
        ShellType::Bash => "bash",
        ShellType::Zsh => "zsh",
        ShellType::Fish => "fish",
        ShellType::PowerShell => "powershell",
        ShellType::Elvish => "elvish",
    }
}

/// Print installation instructions for the generated completions.
pub fn print_install_instructions(shell: ShellType) {
    match shell {
        ShellType::Bash => {
            eprintln!("\n# Installation instructions for bash:");
            eprintln!("# Option 1: User installation");
            eprintln!("mkdir -p ~/.local/share/bash-completion/completions");
            eprintln!("br completions bash > ~/.local/share/bash-completion/completions/br");
            eprintln!("\n# Option 2: System-wide (requires sudo)");
            eprintln!("sudo br completions bash > /etc/bash_completion.d/br");
            eprintln!("\n# Then restart your shell or run: source ~/.bashrc");
        }
        ShellType::Zsh => {
            eprintln!("\n# Installation instructions for zsh:");
            eprintln!("# Option 1: User installation");
            eprintln!("mkdir -p ~/.zsh/completions");
            eprintln!("br completions zsh > ~/.zsh/completions/_br");
            eprintln!("# Add to ~/.zshrc: fpath=(~/.zsh/completions $fpath)");
            eprintln!("\n# Option 2: Oh My Zsh");
            eprintln!("br completions zsh > ~/.oh-my-zsh/completions/_br");
            eprintln!("\n# Then restart your shell or run: exec zsh");
        }
        ShellType::Fish => {
            eprintln!("\n# Installation instructions for fish:");
            eprintln!("mkdir -p ~/.config/fish/completions");
            eprintln!("br completions fish > ~/.config/fish/completions/br.fish");
            eprintln!("\n# Completions are loaded automatically on next fish session");
        }
        ShellType::PowerShell => {
            eprintln!("\n# Installation instructions for PowerShell:");
            eprintln!("# Add to your PowerShell profile ($PROFILE):");
            eprintln!("br completions powershell | Out-String | Invoke-Expression");
        }
        ShellType::Elvish => {
            eprintln!("\n# Installation instructions for elvish:");
            eprintln!("mkdir -p ~/.elvish/lib");
            eprintln!("br completions elvish > ~/.elvish/lib/br.elv");
            eprintln!("# Add to ~/.elvish/rc.elv: use br");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::info;

    fn init_logging() {
        crate::logging::init_test_logging();
    }

    fn render_shell(shell: ShellType) -> String {
        let cmd = Cli::command();
        let mut output = Vec::new();
        write_dynamic_completions(shell, &cmd, &mut output).expect("write dynamic completions");
        String::from_utf8(output).expect("utf8")
    }

    #[test]
    fn test_bash_completion_generation() {
        init_logging();
        info!("test_bash_completion_generation: starting");
        let script = render_shell(ShellType::Bash);

        // Verify basic structure
        assert!(
            script.contains("complete -o"),
            "should contain bash complete command"
        );
        assert!(script.contains("obr"), "should reference obr command");
        assert!(
            script.contains("_clap_complete_obr"),
            "should define dynamic completion function"
        );
        info!("test_bash_completion_generation: assertions passed");
    }

    #[test]
    fn test_zsh_completion_generation() {
        init_logging();
        info!("test_zsh_completion_generation: starting");
        let script = render_shell(ShellType::Zsh);

        assert!(script.contains("#compdef obr"), "should start with #compdef");
        assert!(
            script.contains("_clap_dynamic_completer_obr"),
            "should define dynamic completion function"
        );
        info!("test_zsh_completion_generation: assertions passed");
    }

    #[test]
    fn test_fish_completion_generation() {
        init_logging();
        info!("test_fish_completion_generation: starting");
        let script = render_shell(ShellType::Fish);

        assert!(
            script.contains("complete --keep-order"),
            "should use fish completion registration"
        );
        assert!(script.contains("COMPLETE=fish"), "should set COMPLETE env");
        info!("test_fish_completion_generation: assertions passed");
    }
}
