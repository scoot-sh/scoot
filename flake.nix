{
  # On Linux this flake builds the whole compositor; on macOS the compositor
  # is cfg'd out and the same package is just `scoot msg`, the
  # remote-control client — so the top level names both, and the package's
  # `meta.description` below says per system which one it is. (This has to
  # stay a string literal: the flake loader rejects anything else here.)
  description = "scoot: a scrolling-tiling Wayland compositor that runs without a GPU (on macOS, the remote-control client only)";

  # Pinned to the same nixpkgs revision as vm/, so the dev shell and the VM
  # agree on every library and nothing is downloaded twice.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/8ce4ef6cb6f871616146b9fe26d2a5ae594e94fe";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-linux"
        "x86_64-linux"
        "aarch64-darwin"
        # No `x86_64-darwin`: the pinned nixpkgs (26.11) dropped that
        # platform entirely -- `legacyPackages.x86_64-darwin` throws at
        # eval, so no per-system definition can even be reached. Keeping
        # it would mean repinning the whole tree (and vm/ with it) to
        # 26.05, whose security fixes end with 2026, for a platform Apple
        # itself discontinued. Nothing Intel-specific blocks the client:
        # the Darwin build shares one arch-independent `cfg(not(target_os
        # = "linux"))` path, so `cargo build` from source still works on
        # an Intel Mac -- it is just outside what this flake provides.
      ];
      forEach = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      # The Linux `meta.description` below reads the compositor crate's own
      # metadata (same `readFile`/`fromTOML` shape as `version`), so those
      # two can't drift; the flake-level `description` above has to stay a
      # hand-written literal (see its comment).
      crateDescription =
        (builtins.fromTOML (builtins.readFile ./crates/scoot/Cargo.toml)).package.description;
    in
    {
      # `nix build` / `nix run`, for getting the binary without a dev shell.
      # On Linux this is the whole compositor; on Darwin the compositor is
      # cfg'd out of the crate and what builds is `scoot msg`, the client
      # that drives a compositor running elsewhere (a VM) over its socket.
      # `scoot --headless` there exits with a message saying exactly that,
      # so the same package is honest on both -- see README's Building.
      packages = forEach (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "scoot";
          # Read from where the version already lives, so the two can't drift.
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
          # Scoped to exactly what the build reads, so doc-only edits
          # (README/ROADMAP/CLAUDE, docs/, vm/, scripts/) -- and, worse,
          # the whole working-tree copy this replaced, which dragged
          # target/ (~1GB in-store) and .git along -- no longer bust the
          # derivation's cache and force a full rebuild. Everything else
          # this flake reads at eval time (./Cargo.toml for `version`,
          # ./crates/scoot/Cargo.toml for the description,
          # ./Cargo.lock for `cargoLock.lockFile`,
          # ./vm/compositor-deps.nix for buildInputs) resolves against
          # the flake tree, not `src`, so it stays out of the filter.
          # Verified to cover the build: no build.rs outside crates/, no
          # include_str!/include_bytes! of a root-level file, no
          # .cargo/config or toolchain file, and `license.workspace`
          # is a string, not a file read.
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./crates
            ];
          };

          cargoLock = {
            lockFile = ./Cargo.lock;
            # Smithay is a git dependency pinned by rev (see
            # crates/scoot/Cargo.toml), and a git source carries no
            # crates.io checksum to vendor against, so its tree hash has to
            # be recorded here. Bumping that rev changes the hash and fails
            # the build loudly, printing the one it got -- it cannot drift
            # out of sync quietly. Nothing else in Cargo.lock comes from git.
            outputHashes = {
              "smithay-0.7.0" = "sha256-fptVzfBHApVohO2yvTxbsXGxKHiJ5Brk84x4YkHtp6k=";
            };
          };

          nativeBuildInputs = [ pkgs.pkg-config ];
          # The same list the dev shell uses, Linux-only for the same reason:
          # nothing outside the compositor links against them. The dev shell's
          # LIBRARY_PATH hook below needs no counterpart here -- in a
          # derivation the stdenv cc wrapper puts every buildInput on the link
          # path through NIX_LDFLAGS, which is what those -sys crates' bare
          # -lfoo resolves against. Checked by building, not by assuming.
          buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
            import ./vm/compositor-deps.nix pkgs
          );

          # The workspace's release profile sets `strip = true`, but nixpkgs'
          # cargoBuildHook exports CARGO_PROFILE_RELEASE_STRIP=false to hand
          # stripping to stdenv -- which by default takes debug info only and
          # leaves the symbol table, so the binary would ship 1,224,312 bytes
          # of `.symtab`/`.strtab` the profile asks it not to (4,843,776
          # against 3,619,464, measured here). This puts the profile back.
          # `cargo-auditable`'s non-allocated `.dep-v0` section survives
          # `strip -s`, checked on the built binary, so the dependency
          # manifest `nix build` embeds is not lost with it.
          stripAllList = [ "bin" ];

          # No test run here, deliberately, for two measured reasons. This
          # workspace's release profile sets `panic = "abort"`, which cargo
          # ignores for test targets (a test harness has to unwind), so
          # `cargo test --release` rebuilds the whole dependency tree a
          # second time: on Darwin, where that tree is smallest, 10 crates
          # recompile after a complete `cargo build --release` against 3 (the
          # workspace's own) with `panic = "abort"` removed -- and on Linux
          # the tree it would rebuild includes Smithay. On top of that the
          # compositor's tests bind a real wayland socket and so need a
          # writable `$XDG_RUNTIME_DIR`, which the build sandbox has no
          # reason to provide. `nix build` is the path to a binary; `cargo
          # test` in `nix develop` is where the suite runs.
          doCheck = false;

          meta = {
            # Linux builds the whole compositor, so the crate's own
            # description is the honest one; on Darwin the compositor is
            # cfg'd out and the package is just `scoot msg` (see the
            # comment on `packages` above and README's Building), so the
            # metadata says that instead of advertising a compositor macOS
            # never runs.
            description =
              if pkgs.stdenv.hostPlatform.isDarwin then
                "Remote-control client (`scoot msg`) for the scoot scrolling-tiling Wayland compositor"
              else
                crateDescription;
            homepage = "https://github.com/scoot-sh/scoot";
            license = pkgs.lib.licenses.mit;
            mainProgram = "scoot";
            platforms = pkgs.lib.platforms.linux ++ pkgs.lib.platforms.darwin;
          };
        };
      });

      # So `nix run . -- --headless -- foot` and `nix run . -- msg windows`
      # work. `scoot` is the package's mainProgram, so `nix run` would find
      # it either way; naming it here keeps that explicit rather than
      # implied.
      apps = forEach (pkgs: {
        default = {
          type = "app";
          program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
      });

      devShells = forEach (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = [
            pkgs.rustc
            pkgs.cargo
            pkgs.clippy
            pkgs.rustfmt
            # Part of the documented verification set (see `CLAUDE.md`), so
            # the dev shell has to provide it. Already in the VM's own system
            # closure, which keeps the Linux shell a subset of it as below.
            pkgs.cargo-nextest
            pkgs.pkg-config
          ]
          # Not on Linux: the VM's system closure deliberately drops
          # rust-analyzer to keep erofs packing fast, and `nix develop` there
          # should stay a subset of that closure -- nothing new to fetch.
          ++ pkgs.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.rust-analyzer;
          # The compositor's C dependencies exist only on Linux; the core, the
          # IPC crate and `scoot msg` build anywhere.
          buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
            import ./vm/compositor-deps.nix pkgs
          );

          # `nix develop`'s setup hooks already add -L for every buildInput, so
          # this shellHook only matters inside the dev shell itself: a couple
          # of the compositor's -sys crates (xkbcommon-sys, pixman-sys) probe
          # via pkg-config for cflags but link with a bare -lfoo, which has
          # nothing to find on NixOS without an explicit LIBRARY_PATH. (The
          # VM's system profile hits the same gap outside any dev shell --
          # that's fixed separately, in configuration.nix.)
          shellHook = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
            export LIBRARY_PATH="''${LIBRARY_PATH:+$LIBRARY_PATH:}${pkgs.lib.makeLibraryPath (import ./vm/compositor-deps.nix pkgs)}"
          '';
        };
      });

      formatter = forEach (pkgs: pkgs.nixfmt-rfc-style);
    };
}
