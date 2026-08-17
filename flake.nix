# Nix flake for obr - Agent-first issue tracker
#
# Usage:
#   nix build              Build the obr binary
#   nix run                Run obr directly
#   nix develop            Enter development shell
#
{
  description = "obr - Agent-first issue tracker (SQLite + Org-mode)";

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

        # Nightly Rust toolchain via rust-overlay (required by rust-toolchain.toml;
        # the flake.lock pins the exact nightly for reproducibility).
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

        # Keep tracker-only changes (especially docs/PLAN.org) out of the
        # release derivation. These are the complete compiler inputs; the three
        # documentation files are embedded with include_str!.
        obrSource = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./Cargo.lock
            ./Cargo.toml
            ./README.md
            ./build.rs
            ./docs/AGENT_INTEGRATION.md
            ./docs/CLI_REFERENCE.md
            ./src
          ];
        };

        # Build the obr binary using the nightly Rust toolchain.
        # No bindgenHook / sqlite build inputs: the database engine is fsqlite,
        # pure Rust — nothing links against a C SQLite.
        obr = rustPlatform.buildRustPackage {
          pname = "obr";
          # Read from Cargo.toml rather than repeating it: the binary bakes in
          # CARGO_PKG_VERSION at compile time, so a literal here is a second
          # source of truth that silently drifts on every bump (it did — the
          # 0.2.22 -> 0.2.22+1 change left this behind).
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

          src = obrSource;

          cargoLock = {
            lockFile = "${obrSource}/Cargo.lock";
            outputHashes = {
              # The one git dependency (pinned by rev in Cargo.toml).
              "org2jsonl-0.1.0" = "sha256-mWeouJ5jYN5Cfk5ofb82uIyne7A9SMydpvGszeicHhI=";
            };
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            pkgs.libiconv
            pkgs.apple-sdk_15
          ];

          doCheck = false;

          postInstall = ''
            install -Dm644 LICENSE "$out/share/licenses/beads_rust/LICENSE"
          '';

          meta = with pkgs.lib; {
            description = "Agent-first issue tracker (SQLite + Org-mode)";
            homepage = "https://github.com/jwiegley/obr";
            # Mirrors the LICENSE file in the tree: upstream's MIT plus its
            # rider, kept as-is; contributions made in this fork are MIT.
            # `licenses.mit` is the closest nixpkgs identifier and does not
            # capture the rider — LICENSE is authoritative, and packaging
            # installs it verbatim. See docs/RESIDUALS.md ("Licensing").
            license = licenses.mit;
            mainProgram = "obr";
            platforms = platforms.unix;
          };
        };

        # Shared attributes for check derivations that build from source
        checkCommon = {
          pname = "obr-check";
          version = obr.version;
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "org2jsonl-0.1.0" = "sha256-mWeouJ5jYN5Cfk5ofb82uIyne7A9SMydpvGszeicHhI=";
            };
          };

          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            pkgs.libiconv
            pkgs.apple-sdk_15
          ];

          doCheck = false;
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
            cargo-fuzz
            cargo-audit
            lefthook
            shellcheck
            shfmt
            jq

            # SQLite inspection (the on-disk format is SQLite even though the
            # engine is pure-Rust fsqlite)
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

            # Deliberately no RUST_LOG: a bare level REPLACES the tuned default
            # in src/logging.rs (`obr=debug,fsqlite=error`), so fsqlite's
            # per-statement telemetry buries every command -- `obr init` goes
            # from 3 lines to ~480, and `obr doctor` warns about it. Unset,
            # debug builds still get obr=debug; scope a one-off the way CI
            # does (RUST_LOG=obr=debug).
            echo "obr dev shell - Rust $(rustc --version | cut -d' ' -f2)"
          '';
        };

        # nix build / nix build .#obr
        packages = {
          default = obr;
          inherit obr;
        };

        # nix run
        apps.default = flake-utils.lib.mkApp {
          drv = obr;
          name = "obr";
        };

        # nix flake check
        #
        # Deliberately only build + formatting: upstream's tree is red at
        # baseline under both clippy (85 first-party pedantic/nursery errors)
        # and `cargo test --lib --bins` (266 failures + 4 process-aborting
        # tests), so sandboxed clippy/test checks would fail unconditionally.
        # Lint and unit regressions are gated by the no-NEW-failures scripts
        # (scripts/lint-gate.sh, scripts/unit-gate.sh) used by lefthook and CI
        # instead. Do not add a check that is known-red.
        checks = {
          # Verify the package builds
          build = self.packages.${system}.default;

          # The release source must contain real compiler inputs while keeping
          # the mutable tracker surface out of the package hash.
          source-filter = pkgs.runCommand "obr-source-filter" { } ''
            test -f ${obrSource}/src/main.rs
            test -f ${obrSource}/build.rs
            test -f ${obrSource}/docs/CLI_REFERENCE.md
            test ! -e ${obrSource}/docs/PLAN.org
            test ! -e ${obrSource}/tests
            touch $out
          '';

          # Check source formatting with cargo fmt
          formatting = rustPlatform.buildRustPackage (checkCommon // {
            pname = "obr-fmt";

            buildPhase = ''
              cargo fmt --all -- --check
            '';

            installPhase = ''
              mkdir -p $out
            '';
          });
        };
      });
}
