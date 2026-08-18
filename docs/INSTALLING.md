# Installation Guide

Complete installation instructions for `obr` (obr), including all supported platforms and methods.

---

## Table of Contents

- [Requirements](#requirements)
- [Quick Install](#quick-install)
- [Installation Methods](#installation-methods)
  - [Cargo Install (Recommended)](#cargo-install-recommended)
  - [Build from Source](#build-from-source)
  - [Pre-built Binaries](#pre-built-binaries)
- [Platform-Specific Notes](#platform-specific-notes)
  - [Linux](#linux)
  - [macOS](#macos)
  - [Windows](#windows)
- [Configuration](#configuration)
- [Verifying Installation](#verifying-installation)
- [Proxy Configuration](#proxy-configuration)
- [Troubleshooting](#troubleshooting)

---

## Requirements

### Minimum Requirements

- **Rust**: Nightly toolchain (required for Rust 2024 edition features)
- **SQLite**: Bundled (no system SQLite required)
- **Git**: Optional (for version control integration)

### Supported Platforms

| Platform | Architecture | Status |
|----------|--------------|--------|
| Linux | x86_64 | Fully supported |
| Linux | aarch64 (ARM64) | Fully supported |
| macOS | x86_64 (Intel) | Fully supported |
| macOS | aarch64 (Apple Silicon) | Fully supported |
| Windows | x86_64 | Supported |

---

## Quick Install

### One-liner (Cargo)

```bash
cargo install --git https://github.com/jwiegley/obr.git obr --locked
```

### One-liner (Build from Source)

```bash
git clone https://github.com/jwiegley/obr.git && cd obr && cargo build --release && sudo cp target/release/obr /usr/local/bin/
```

---

## Installation Methods

### Cargo Install (Recommended)

The simplest method using Rust's package manager:

```bash
cargo install --git https://github.com/jwiegley/obr.git obr --locked
```

Keep the explicit `obr` package selector and `--locked` on every git
source install. The selector avoids ambiguity with the repository's fuzz
package, while `--locked` uses the dependency versions validated against
obr's pinned nightly instead of resolving a newer, potentially
incompatible graph.

**Requirements:**
- Rust nightly toolchain

**Install Rust nightly:**

```bash
# Install rustup if not present
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install and set nightly as default
rustup install nightly
rustup default nightly

# Or use nightly for just this install
rustup run nightly cargo install --git https://github.com/jwiegley/obr.git obr --locked
```

### Build from Source

For development or customization:

```bash
# Clone the repository
git clone https://github.com/jwiegley/obr.git
cd obr

# Build release binary (optimized for size)
cargo build --release

# The binary is at ./target/release/obr
./target/release/obr --version

# Optional: Install system-wide
sudo cp target/release/obr /usr/local/bin/
# Or for user-local install
cp target/release/obr ~/.local/bin/
```

**Build Options:**

```bash
# Build with all features
cargo build --release --all-features

# Build with debug symbols (for development)
cargo build

# Run tests before building
cargo test && cargo build --release
```

### Pre-built Binaries

Pre-built binaries are available from GitHub Releases:

A release URL has two version-shaped parts and they are not the same string:

- the **tag** (`v0.5.7+1`) — the true version, `+` and all;
- the **asset name** (`obr-0.5.7.1-…`) — the same version with the `+`
  flattened to `.`, because GitHub rewrites a `+` in an uploaded asset name.

```bash
TAG=v0.5.7+1   # the release tag: the version verbatim
ASSET=0.5.7.1  # the same version with '+' flattened to '.'

# Example for Linux x86_64
curl -L "https://github.com/jwiegley/obr/releases/download/${TAG}/obr-${ASSET}-linux_amd64.tar.gz" -o obr.tar.gz
tar -xzf obr.tar.gz obr
sudo install -m 0755 obr /usr/local/bin/obr

# Example for macOS ARM64
curl -L "https://github.com/jwiegley/obr/releases/download/${TAG}/obr-${ASSET}-darwin_arm64.tar.gz" -o obr.tar.gz
tar -xzf obr.tar.gz obr
sudo install -m 0755 obr /usr/local/bin/obr
```

`install.sh` derives both from one `--version` argument, so
`./install.sh --version v0.5.7+1` needs neither spelling.

**Verify Checksum:**

```bash
# Download checksum file
curl -L https://github.com/jwiegley/obr/releases/latest/download/checksums.sha256 -o checksums.sha256

# Verify (Linux)
sha256sum -c checksums.sha256 --ignore-missing

# Verify (macOS)
shasum -a 256 -c checksums.sha256 --ignore-missing
```

---

## Platform-Specific Notes

### Linux

**Ubuntu/Debian:**

```bash
# Install build dependencies
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev

# Install Rust nightly
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup install nightly
rustup default nightly

# Install obr
cargo install --git https://github.com/jwiegley/obr.git obr --locked
```

**Fedora/RHEL:**

```bash
# Install build dependencies
sudo dnf install -y gcc pkg-config openssl-devel

# Install Rust and obr
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup install nightly
rustup default nightly
cargo install --git https://github.com/jwiegley/obr.git obr --locked
```

**Arch Linux:**

```bash
# Install dependencies
sudo pacman -S rust

# Install obr
rustup install nightly
rustup default nightly
cargo install --git https://github.com/jwiegley/obr.git obr --locked
```

### macOS

**With Homebrew (Rust installation):**

```bash
# Install Rust via rustup (recommended over Homebrew Rust)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install nightly
rustup install nightly
rustup default nightly

# Install obr
cargo install --git https://github.com/jwiegley/obr.git obr --locked
```

**Apple Silicon (M1/M2/M3):**

No special steps needed. The build automatically targets the native architecture.

```bash
# Verify you're building for ARM64
rustc --print target-list | grep aarch64-apple-darwin
```

### Windows

**With PowerShell:**

```powershell
# Install Rust via rustup
Invoke-WebRequest -Uri https://win.rustup.rs/x86_64 -OutFile rustup-init.exe
.\rustup-init.exe

# Restart PowerShell, then:
rustup install nightly
rustup default nightly

# Install obr
cargo install --git https://github.com/jwiegley/obr.git obr --locked
```

**With WSL2 (Recommended for Windows):**

```bash
# In WSL2 (Ubuntu)
sudo apt update && sudo apt install -y build-essential
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup install nightly
rustup default nightly
cargo install --git https://github.com/jwiegley/obr.git obr --locked
```

---

## Configuration

After installation, obr works out of the box. Optional configuration:

### Initialize in a Project

```bash
cd your-project
obr init
```

This creates:
- `.obr/obr.db` - SQLite database
- `.obr/metadata.json` - Configuration metadata

### User Configuration

Create `~/.config/obr/config.yaml` for global defaults:

```yaml
# Default issue prefix
prefix: bd

# Default priority for new issues (0-4)
default_priority: 2

# Default issue type
default_type: task

# Auto-flush after mutations
auto_flush: true
```

### Project Configuration

Create `.obr/config.yaml` for project-specific settings:

```yaml
# Project-specific prefix
prefix: myproj

# Override defaults
default_priority: 1
```

---

## Verifying Installation

```bash
# Check version
obr version

# Expected output:
# obr 0.1.0 (abc1234)
# Built: 2026-01-17

# Check help
obr --help

# Run a simple command
obr init
obr create "Test issue" --type task
obr list
obr delete bd-xxx  # Clean up test issue
```

---

## Updating

`obr` has no self-updater and no `upgrade` subcommand — it is an unpublished
fork with no release feed to check against. Re-run the install command to pick
up a newer commit:

```bash
cargo install --git https://github.com/jwiegley/obr.git obr --locked --force
```

---

## Proxy Configuration

For users behind corporate proxies:

### Environment Variables

```bash
# HTTP proxy
export HTTP_PROXY=http://proxy.example.com:8080
export HTTPS_PROXY=http://proxy.example.com:8080

# For cargo operations
export CARGO_HTTP_PROXY=http://proxy.example.com:8080

# No proxy for local addresses
export NO_PROXY=localhost,127.0.0.1
```

### Cargo Configuration

Create or edit `~/.cargo/config.toml`:

```toml
[http]
proxy = "http://proxy.example.com:8080"

[https]
proxy = "http://proxy.example.com:8080"
```

---

## Troubleshooting

### Common Issues

#### "error: could not find `Cargo.toml`"

Make sure you're running the cargo install command, not trying to build from a non-existent local directory:

```bash
# Correct: install from git
cargo install --git https://github.com/jwiegley/obr.git obr --locked

# Wrong: trying to build without cloning first
cargo build  # This requires Cargo.toml in current directory
```

#### "error[E0658]: edition 2024 is unstable"

You need the Rust nightly toolchain:

```bash
rustup install nightly
rustup default nightly
# Or use: rustup run nightly cargo install --git \
#   https://github.com/jwiegley/obr.git --locked
```

#### "error: linker `cc` not found"

Install build tools:

```bash
# Ubuntu/Debian
sudo apt install build-essential

# Fedora
sudo dnf install gcc

# macOS
xcode-select --install
```

#### "permission denied" when installing to /usr/local/bin

Either use sudo or install to a user directory:

```bash
# Option 1: Use sudo
sudo cp target/release/obr /usr/local/bin/

# Option 2: Install to user directory
mkdir -p ~/.local/bin
cp target/release/obr ~/.local/bin/
# Add to PATH if needed:
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
```

#### Database Lock Errors

If you see "database is locked" errors:

```bash
# Check for stale locks
ls -la .obr/*.db-*

# Remove stale lock files (only if obr is not running)
rm .obr/*.db-shm .obr/*.db-wal .obr/*.db-journal
```

### Getting Help

- **Documentation**: [README.md](../README.md)
- **Troubleshooting**: [docs/TROUBLESHOOTING.md](./TROUBLESHOOTING.md)
- **Issues**: [GitHub Issues](https://github.com/jwiegley/obr/issues)

---

## Related Documentation

- [README.md](../README.md) - Project overview
- [AGENTS.md](../AGENTS.md) - Agent integration guidelines
- [CLI_REFERENCE.md](./CLI_REFERENCE.md) - Complete command reference
- [ARCHITECTURE.md](./ARCHITECTURE.md) - Technical architecture
- [TROUBLESHOOTING.md](./TROUBLESHOOTING.md) - Common issues and solutions

---

*Last updated: 2026-01-17*
