# Nix flake for obr - Agent-first issue tracker
#
# Usage:
#   nix build              Build the obr binary
#   nix run                Run obr directly
#   nix develop            Enter development shell
#   nix flake check        Build, formatting, compiler identity, unit gate, Clippy
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

        # Cargo, Nix builds, and checks use the same pinned nightly. The overlay
        # lock must contain this date; it must never silently select latest.
        toolchainChannel = (builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)).toolchain.channel;
        rustToolchain = (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override (args: {
          extensions = args.extensions ++ [
            "rust-src"
            "rust-analyzer"
          ];
        });

        # Custom rustPlatform using the nightly toolchain
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        # Keep tracker-only changes (especially docs/PLAN.org) out of the
        # release derivation. These are the complete compiler inputs; the three
        # documentation files are embedded with include_str!.
        obrFiles = pkgs.lib.fileset.unions [
          ./Cargo.lock
          ./Cargo.toml
          ./README.md
          ./LICENSE
          ./build.rs
          ./docs/AGENT_INTEGRATION.md
          ./docs/CLI_REFERENCE.md
          ./src
        ];
        obrSource = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = obrFiles;
        };

        # All-target linting needs every Rust test and benchmark; the unit gate
        # also reads integration snapshots and the committed failure inventories.
        checkSource = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            obrFiles
            ./rust-toolchain.toml
            ./tests
            ./benches
            ./ci/baseline
            ./scripts/unit-gate.sh
            ./scripts/lint-gate.sh
          ];
        };

        # Build the obr binary using the nightly Rust toolchain.
        # No bindgenHook / sqlite build inputs: the database engine is fsqlite,
        # pure Rust — nothing links against a C SQLite.
        obr = rustPlatform.buildRustPackage {
          pname = "obr";
          # Read from Cargo.toml rather than repeating it: the binary bakes in
          # CARGO_PKG_VERSION at compile time, so a literal here is a second
          # source of truth that silently drifts on every bump.
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

          # The release build does not run tests. `nix flake check` runs the
          # source-owned baseline-unit and baseline-lint gates below.
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
          src = checkSource;

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
          # Custom gates retain the standard Cargo hooks' build/test job budget.
          preCheck = ''
            export CARGO_BUILD_JOBS="$NIX_BUILD_CORES"
            export RUST_TEST_THREADS="$NIX_BUILD_CORES"
          '';
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
        # The unit gate accepts no new unit failures and runs its curated
        # integration set, which has no failure inventory. Clippy is strict;
        # a test-only mutant verifies that the real lint gate executes it.
        checks = {
          # Verify the package builds
          build = self.packages.${system}.default;

          # The release source must contain real compiler inputs while keeping
          # the mutable tracker surface out of the package hash.
          source-filter = pkgs.runCommand "obr-source-filter" { } ''
            test -f ${obrSource}/src/main.rs
            test -f ${obrSource}/build.rs
            test -f ${obrSource}/docs/CLI_REFERENCE.md
            test -f ${obrSource}/LICENSE
            test ! -e ${obrSource}/docs/PLAN.org
            test ! -e ${obrSource}/tests
            test -f ${checkSource}/tests/vocabulary_lint.rs
            test -f ${checkSource}/tests/golden_obr_init.rs
            test -f ${checkSource}/tests/golden_rich_panels.rs
            test -d ${checkSource}/tests/snapshots
            test -f ${checkSource}/ci/baseline/vocabulary-debt.txt
            test -f ${checkSource}/scripts/unit-gate.sh
            test -f ${checkSource}/scripts/lint-gate.sh
            test ! -e ${checkSource}/docs/PLAN.org
            test ! -e ${checkSource}/.obr
            touch $out
          '';

          # rustc's commit date can precede the nightly distribution date.
          # Compare its actual release with the overlay's version and the
          # requested distribution date, and retain the full compiler identity.
          toolchain-identity = pkgs.runCommand "obr-toolchain-identity" { } ''
            ${rustToolchain}/bin/rustc --version --verbose > "$out"
            release=$(sed -n 's/^release: //p' "$out")
            test ${pkgs.lib.escapeShellArg rustToolchain.version} = \
              "$release"-${pkgs.lib.escapeShellArg (pkgs.lib.removePrefix "nightly-" toolchainChannel)}
          '';

          baseline-unit = rustPlatform.buildRustPackage (checkCommon // {
            pname = "obr-baseline-unit";
            dontBuild = true;
            doCheck = true;
            nativeCheckInputs = with pkgs; [ git sqlite unixtools.script ];
            checkPhase = ''
              runHook preCheck
              bash scripts/unit-gate.sh
              runHook postCheck
            '';
            installPhase = ''
              mkdir -p "$out"
            '';
          });

          baseline-lint = rustPlatform.buildRustPackage (checkCommon // {
            pname = "obr-baseline-lint";
            # cargo-auditable replaces Clippy's RUSTC_WORKSPACE_WRAPPER.
            auditable = false;
            dontBuild = true;
            doCheck = true;
            checkPhase = ''
              runHook preCheck
              bash scripts/lint-gate.sh
              runHook postCheck
            '';
            installPhase = ''
              mkdir -p "$out"
            '';
          });

          clippy-execution = self.checks.${system}.baseline-lint.overrideAttrs (old: {
            pname = "obr-clippy-execution";
            # eq_op exempts #[test] functions; use a helper compiled only for tests.
            postPatch = (old.postPatch or "") + ''
              cat >> src/lib.rs <<'EOF'

              #[cfg(test)]
              mod nix_clippy_execution_probe {
                  #[allow(dead_code)]
                  fn equal_operands() -> bool {
                      1 == 1
                  }
              }
              EOF
            '';
            checkPhase = ''
              runHook preCheck
              if bash scripts/lint-gate.sh > clippy-execution.log 2>&1; then
                cat clippy-execution.log >&2
                echo "clippy-execution: the test-only lint was accepted" >&2
                exit 1
              fi
              if ! grep -F 'clippy::eq_op' clippy-execution.log; then
                cat clippy-execution.log >&2
                echo "clippy-execution: missing the injected Clippy diagnostic" >&2
                exit 1
              fi
              runHook postCheck
            '';
            installPhase = ''
              mkdir -p "$out"
              cp clippy-execution.log "$out/"
            '';
          });

          # Check source formatting with cargo fmt
          formatting = rustPlatform.buildRustPackage (checkCommon // {
            pname = "obr-fmt";

            buildPhase = ''
              cargo-fmt --all -- --check
            '';

            installPhase = ''
              mkdir -p $out
            '';
          });
        };
      });
}
