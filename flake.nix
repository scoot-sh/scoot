{
  # On Linux this flake builds the whole compositor; on macOS the default is
  # `scootctl`, the remote-control client -- so the top level names both, and
  # each package's `meta.description` below comes from its own crate. (The
  # flake-level `description` here has to stay a string literal: the flake
  # loader rejects anything else.)
  description = "scoot: a scrolling-tiling Wayland compositor that runs without a GPU (on macOS, the scootctl remote-control client only)";

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
      # metadata (same `readFile`/`fromTOML` shape as `version`), and the
      # client's reads its crate's the same way, so no description can drift
      # from the crate it names; the flake-level `description` above has to
      # stay a hand-written literal (see its comment).
      crateDescription =
        (builtins.fromTOML (builtins.readFile ./crates/scoot/Cargo.toml)).package.description;
      scootctlDescription =
        (builtins.fromTOML (builtins.readFile ./crates/scootctl/Cargo.toml)).package.description;
    in
    {
      # `nix build` / `nix run`, for getting the binaries without a dev shell.
      # `scoot` is the whole compositor (plus the `scoot msg` client alias),
      # built `-p scoot` so `$out/bin` carries only the `scoot` binary;
      # `scootctl` is the standalone remote-control client that drives a
      # compositor running elsewhere (a VM) over its socket. On Linux the
      # default is the compositor; on Darwin the compositor is cfg'd out of
      # the crate, so the default is the client -- and `scoot --headless`
      # there exits with a message saying exactly that, so the same package
      # is honest on both (see README's Install section).
      packages = forEach (
        pkgs:
        let
          # Read from where the version already lives, so the two can't drift.
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
          # Scoped to exactly what the build reads, so doc-only edits
          # (README/ROADMAP/CLAUDE, docs/, vm/, scripts/) -- and, worse,
          # the whole working-tree copy this replaced, which dragged
          # target/ (~1GB in-store) and .git along -- no longer bust the
          # derivation's cache and force a full rebuild. Everything else
          # this flake reads at eval time (./Cargo.toml for `version`,
          # ./crates/scoot/Cargo.toml and ./crates/scootctl/Cargo.toml for
          # the descriptions, ./Cargo.lock for `cargoLock.lockFile`,
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

          scoot = pkgs.rustPlatform.buildRustPackage {
            pname = "scoot";
            inherit version src cargoLock;

            # Just this crate, not the whole workspace: `$out/bin` carries
            # only `scoot` (the `scoot msg` alias is part of that binary,
            # not a second one) -- the mirror of the `scootctl` package's
            # flag below. Without this, `buildRustPackage` builds the
            # whole workspace and ships a redundant `scootctl` (gh #172).
            # Behavior-preserving for the shipped binary: the `scootctl`
            # binary unit enables no features on any shared crate (it
            # depends on bare `scoot-ipc` plus `serde_json`), so the
            # `scoot` unit graph is identical either way.
            cargoBuildFlags = [
              "-p"
              "scoot"
            ];

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

            # No test run here, deliberately, for two reasons. This
            # workspace's release profile sets `panic = "abort"`, which cargo
            # ignores for test targets (a test harness has to unwind), so
            # `cargo test --release` rebuilds the whole dependency tree a
            # second time -- and on Linux the tree it would rebuild includes
            # Smithay. On top of that the compositor's tests bind a real
            # wayland socket and so need a writable `$XDG_RUNTIME_DIR`, which
            # the build sandbox has no reason to provide. `nix build` is the
            # path to a binary; `cargo test` in `nix develop` is where the
            # suite runs.
            doCheck = false;

            meta = {
              # Linux builds the whole compositor, so the crate's own
              # description is the honest one; on Darwin the compositor is
              # cfg'd out and the package is just `scoot msg` (see the
              # comment on `packages` above and README's Install section), so the
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

          scootctl = pkgs.rustPlatform.buildRustPackage {
            pname = "scootctl";
            inherit version src cargoLock;

            # Just this crate, not the whole workspace: `$out/bin` carries
            # only `scootctl`, and on Darwin nothing compositor-shaped (and
            # no Smithay tree) is compiled at all.
            cargoBuildFlags = [
              "-p"
              "scootctl"
            ];

            # Nothing to probe or link: the client is pure Rust
            # (`scoot-ipc` plus `serde_json` plus std), so neither of the
            # lists the compositor package above needs applies here -- which
            # is also why this package builds anywhere with zero cfg gating.

            # Same profile-put-back as the compositor package above (see its
            # comment): the release profile asks for a full strip and the
            # cargo hook hands stripping to stdenv.
            stripAllList = [ "bin" ];

            # No test run here, for the same reasons as above (the
            # `panic = "abort"` double-build plus no `$XDG_RUNTIME_DIR` in
            # the sandbox). `nix build` is the path to a binary; `cargo
            # test` in `nix develop` is where the suite runs.
            doCheck = false;

            meta = {
              # The crate's own description, no per-system conditional: this
              # package is the client on every system.
              description = scootctlDescription;
              homepage = "https://github.com/scoot-sh/scoot";
              license = pkgs.lib.licenses.mit;
              mainProgram = "scootctl";
              platforms = pkgs.lib.platforms.linux ++ pkgs.lib.platforms.darwin;
            };
          };
        in
        {
          # Linux gets the compositor, Darwin gets the client.
          default = if pkgs.stdenv.hostPlatform.isDarwin then scootctl else scoot;
          inherit scoot scootctl;
        }
      );

      # So `nix run . -- --headless -- foot` and `nix run . -- msg windows`
      # work, and `nix run .#scootctl -- windows` runs the standalone client
      # anywhere. Each `mainProgram` would resolve without the explicit
      # naming; keeping it explicit rather than implied.
      apps = forEach (pkgs: {
        default = {
          type = "app";
          program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
        scootctl = {
          type = "app";
          program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.scootctl;
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
          # IPC crate and the `scootctl` client build anywhere.
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

      # `programs.scoot`: a home-manager module (per-user config file,
      # session script hook, portals.conf install) and a NixOS module
      # (system package + opt-in login-screen session entry). Split the
      # way they are because the config file is per-user while session
      # wiring is system-level -- most compositors ship both, thin.
      # Each wrapper below is one `mkDefault` for `package` over the
      # pure module in `nix/modules/`: without an overlay there is no
      # `pkgs.scoot` to default to, so the flake's own build (the same
      # per-system default as `packages`) is injected here instead, and
      # an explicit setting still wins -- except on Darwin on the HM
      # side (see below). See docs/nix.md.
      homeManagerModules =
        let
          hmWrapper =
            { pkgs, ... }:
            {
              imports = [ ./nix/modules/home.nix ];
              # On Darwin the per-system default would be `scootctl`, a
              # binary not named `scoot` -- while the option reads "The
              # scoot package to install" and docs/nix.md frames the
              # macOS use as config management (the config edited here
              # deploys to a Linux box). So the Darwin default is null:
              # files-only, which the module explicitly supports. Darwin
              # users who want the client set `package` explicitly.
              programs.scoot.package = nixpkgs.lib.mkDefault (
                if pkgs.stdenv.hostPlatform.isDarwin then null else self.packages.${pkgs.system}.default
              );
            };
        in
        {
          default = hmWrapper;
          scoot = hmWrapper;
        };

      # Current home-manager spelling (`homeModules.*`); the legacy
      # `homeManagerModules.*` above keeps working for existing
      # consumers -- both names resolve to the same set.
      homeModules = self.homeManagerModules;

      nixosModules =
        let
          osWrapper =
            { pkgs, ... }:
            {
              imports = [ ./nix/modules/nixos.nix ];
              programs.scoot.package = nixpkgs.lib.mkDefault self.packages.${pkgs.system}.default;
            };
        in
        {
          default = osWrapper;
          scoot = osWrapper;
        };

      # Hermetic module checks (see nix/tests.nix): standalone
      # evalModules + rendered-file content assertions. Eval-time Nix,
      # so no benchmark applies -- nothing here runs per-event or
      # per-frame; it runs once per `nix flake check`.
      checks = forEach (pkgs: {
        scoot-modules = pkgs.callPackage ./nix/tests.nix { };
      });

      formatter = forEach (pkgs: pkgs.nixfmt);
    };
}
