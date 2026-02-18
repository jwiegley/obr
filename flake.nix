# Nix flake for beads_rust - Agent-first issue tracker
#
# Usage:
#   nix build              Build the br binary
#   nix run                Run br directly
#   nix develop            Enter development shell
#
# First time setup:
#   nix flake lock         Generate flake.lock (commit this file)
#
{
  description = "beads_rust - Agent-first issue tracker (SQLite + JSONL)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        # Nightly Rust toolchain via rust-overlay (required for edition 2024)
        rustToolchain = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
            "clippy"
            "rustfmt"
          ];
        };

        # Custom rustPlatform using the nightly toolchain
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        # Build the br binary using the nightly Rust toolchain
        beads_rust = rustPlatform.buildRustPackage {
          pname = "beads_rust";
          version = "0.1.14";

          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "toon_rust-0.1.1" = "sha256-3NUdC0BL7IvB1eP1USrA9oeSTRGpE18fgsw6t1W0oQ8=";
            };
          };

          # Native build inputs for rusqlite bundled SQLite
          nativeBuildInputs = with pkgs; [
            pkg-config
          ] ++ [
            rustPlatform.bindgenHook
          ];

          buildInputs = with pkgs; [
            sqlite
          ] ++ lib.optionals stdenv.hostPlatform.isDarwin [
            libiconv
            apple-sdk_15
          ];

          doCheck = false;

          meta = with pkgs.lib; {
            description = "Agent-first issue tracker (SQLite + JSONL)";
            homepage = "https://github.com/Dicklesworthstone/beads_rust";
            license = licenses.mit;
            mainProgram = "br";
            platforms = platforms.unix;
          };
        };

      in
      {
        # nix develop
        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
          ] ++ (with pkgs; [
            # Build dependencies
            pkg-config

            # Development tools
            cargo-watch
            cargo-edit
            cargo-expand
            cargo-nextest

            # SQLite inspection
            sqlite
          ] ++ lib.optionals stdenv.hostPlatform.isDarwin [
            libiconv
            apple-sdk_15
          ]);

          shellHook = ''
            # Unset DEVELOPER_DIR to avoid conflict between the default stdenv
            # SDK and apple-sdk_15 baked into the clang wrapper.
            unset DEVELOPER_DIR

            export RUST_BACKTRACE=1
            export RUST_LOG=info
            echo "beads_rust dev shell - Rust $(rustc --version | cut -d' ' -f2)"
          '';
        };

        # nix build / nix build .#beads_rust
        packages = {
          default = beads_rust;
          inherit beads_rust;
        };

        # nix run
        apps.default = flake-utils.lib.mkApp {
          drv = beads_rust;
          name = "br";
        };
      });
}
