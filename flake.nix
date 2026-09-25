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
      # `scoot-gpu` is the same binary with the `gpu-scanout` build feature
      # (`--tty --renderer gles` scans out from the GPU instead of reading
      # back; needs OS EGL drivers -- see docs/nix.md);
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
            # Two dependencies come from git, both scoot-sh forks pinned by
            # rev (docs/forks.md): Smithay (crates/scoot/Cargo.toml, upstream
            # `0ff00983` plus one commit) and wayland-backend (the root
            # Cargo.toml's `[patch.crates-io]`, the 0.3.17 release plus one
            # commit). A git source carries no crates.io checksum to vendor
            # against, so each tree hash has to be recorded here. The
            # wayland-backend entry covers `wayland-sys` too (one repository,
            # one fetch) and includes the repository's `wayland-protocols`
            # submodule. Bumping either rev changes its hash and fails the
            # build loudly, printing the one it got -- it cannot drift out of
            # sync quietly. Nothing else in Cargo.lock comes from git.
            outputHashes = {
              "smithay-0.7.0" = "sha256-crzKE9KVKRqffVQH88Mjbcmkc4upo76DJQziNhM+ZNY=";
              "wayland-backend-0.3.17" = "sha256-o818Oati0DTsgXnd2E6Qn67Lokzt4/FELns3rHE2Dlg=";
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

            # Smithay reaches libEGL through `dlopen`, not a link-time
            # `DT_NEEDED`, so the cc wrapper's RUNPATH logic (correct for
            # `-lfoo` above) puts nothing in the RUNPATH for it -- and
            # `--renderer gles` dies in Smithay's ffi with `Failed to load
            # LibEGL` before device enumeration ever runs (gh #177). Forced
            # the way nixpkgs' own niri package does it
            # (`pkgs/by-name/ni/niri/package.nix`: "Force linking with
            # libEGL ... so they can be discovered by `dlopen()`"):
            # `--no-as-needed -lEGL` lands libEGL.so.1 in `DT_NEEDED`, which
            # the loader resolves through the RUNPATH the cc wrapper builds
            # from `buildInputs` -- so the `dlopen` then finds the
            # already-loaded handle. No wrapper script and no
            # `LD_LIBRARY_PATH` leaking into every spawned client;
            # `readelf -d` shows the `NEEDED` entry, which is the audit. A
            # `postFixup` `patchelf --add-rpath` would do the same job with
            # a hand-computed store path; this reuses the wrapper's own
            # path computation instead of duplicating it.
            #
            # Deliberately derivation-only, never an in-tree `RUSTFLAGS` or
            # cargo config: CI's `ldd` gate asserts the plain `cargo build`
            # links no libEGL, and that gate must keep passing. The closure
            # always carries libglvnd, so this costs GPU-free operation
            # nothing: libEGL *loads* everywhere, and what fails on a
            # driverless box is device enumeration -- the designed startup
            # error, reached through the compositor's own pre-flight probe.
            #
            # Mesa's vendor ICDs are NOT bundled here: they come from the
            # host OS's OpenGL setup (on NixOS, `hardware.graphics`), the
            # standard nixpkgs pattern -- bundling Mesa would risk shadowing
            # the host's drivers (notably Asahi's) with wrong ones. So
            # `--renderer gles` from this package needs an OS that provides
            # EGL drivers; see docs/nix.md.
            #
            # Linux-only: these are GNU-ld flags and Apple's ld rejects
            # them (`ld: unknown option: --push-state`), and there is no
            # libEGL to link on Darwin anyway -- the compositor is cfg'd
            # out there, so nothing reaches EGL. (Upstream niri, where this
            # trick comes from, is Linux-only and never hits the question.)
            env.RUSTFLAGS = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
              toString (
                map (arg: "-C link-arg=" + arg) [
                  "-Wl,--push-state,--no-as-needed"
                  "-lEGL"
                  "-Wl,--pop-state"
                ]
              )
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
          # The GPU scanout tier as a package (gh #177): off by default in
          # the build above because `backend_gbm` is a link-time libgbm
          # dependency and GPU-free operation is a fixed decision (see
          # `crates/scoot/Cargo.toml`), opted into here, where linking
          # libgbm is the point. `overrideAttrs` keeps everything the base
          # package sets (`cargoBuildFlags`, `buildInputs` -- libgbm is
          # already in `vm/compositor-deps.nix` -- the EGL link forcing
          # above, stripping, `doCheck`); the binary stays named `scoot`
          # (`mainProgram` unchanged), so this is a second build of the
          # same binary with the scanout tier compiled in. `ldd` on the two
          # is the audit: `scoot-gpu` links libgbm, `scoot` does not.
          #
          # `cargoBuildFeatures`, not `buildFeatures`: the issue proposed
          # the latter, but the build proved it a no-op -- `buildFeatures`
          # is an argument to `buildRustPackage` (consumed when the base
          # derivation is called, turned into `cargoBuildFeatures`), while
          # `overrideAttrs` can only change the resulting derivation's
          # own attrs. Overriding `buildFeatures` built an unfeatured
          # binary (proven by `ldd`: no libgbm); `cargoBuildFeatures` is
          # the attr the cargo hook actually reads
          # (`build-rust-package/default.nix`), and the rebuild log shows
          # `--features=gpu-scanout` in the hook flags.
          # Darwin note: `gpu-scanout` enables Smithay's `backend_gbm`,
          # which only compiles on Linux -- but Smithay itself is a
          # Linux-only dependency of this crate, so on Darwin the feature
          # resolves without building anything new and the package stays
          # the same client-shaped binary. Proven by building, not by
          # assuming; if that ever stops holding, gate this to Linux.
          scoot-gpu = scoot.overrideAttrs (old: {
            pname = "scoot-gpu";
            cargoBuildFeatures = [ "gpu-scanout" ];
            meta = old.meta // {
              # The base description already says the honest per-system
              # thing (compositor on Linux, `scoot msg` client on Darwin);
              # the suffix names the one difference, per system too.
              description =
                old.meta.description
                + (
                  if pkgs.stdenv.hostPlatform.isDarwin then
                    " (gpu-scanout build feature: Linux-only, same client binary here)"
                  else
                    " (gpu-scanout build feature: --tty --renderer gles scans out from the GPU)"
                );
            };
          });
        }
      );

      # So `nix run . -- --headless -- foot` and `nix run . -- msg windows`
      # work, `nix run .#scootctl -- windows` runs the standalone client
      # anywhere, and `nix run .#scoot-gpu -- --tty -- ...` runs the scanout
      # build. Each `mainProgram` would resolve without the explicit
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
        scoot-gpu = {
          type = "app";
          program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.scoot-gpu;
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
          #
          # `LD_LIBRARY_PATH` is a separate problem from `LIBRARY_PATH` above,
          # and only libglvnd goes on it. `LIBRARY_PATH` is link time; Smithay
          # **dlopens** `libEGL.so.1` at *run* time, and a plain `cargo build`
          # deliberately links no libEGL at all (CI's `ldd` gate asserts that,
          # and the package instead link-injects `-lEGL` by derivation-only
          # RUSTFLAGS -- see `packages.scoot` above). NixOS has no global
          # `libEGL.so.1`: `/run/opengl-driver/lib` carries the vendor ICD
          # (`libEGL_mesa.so`), not the dispatch library. So without this, two
          # tests that reach EGL (`render::gles::tests::
          # libegl_loads_where_the_suite_runs` and `render::tests::
          # a_capture_covers_the_whole_target_at_the_backends_own_size`) fail
          # in `nix develop` on any NixOS host -- found on the Asahi M2,
          # 2026-09-21, where `cargo nextest run --workspace` was 2-red for
          # this reason alone while the dev VM stayed green (its *system*
          # profile supplies the path, per configuration.nix) and CI stayed
          # green (it resolves a software EGL).
          #
          # Only libglvnd, never the whole `compositor-deps` list: that list
          # includes `libgbm`/`mesa`, and putting nixpkgs' Mesa ahead of the
          # host's on `LD_LIBRARY_PATH` is the same driver-shadowing hazard
          # `packages.scoot` refuses to risk by bundling ICDs. libglvnd is the
          # vendor-neutral dispatch; it finds the host's real driver through
          # `/run/opengl-driver`, so it shadows no *driver* -- verified live on
          # Asahi, where a `gles` run through this path reaches
          # `GL Renderer: "Apple M2 (G14G B0)"`.
          #
          # It is not, however, true that it shadows *nothing*: glibc resolves
          # `LD_LIBRARY_PATH` ahead of `DT_RUNPATH`, and the export is
          # inherited by every child of the shell, so a GL app launched from
          # `nix develop` (a client the compositor spawns included) gets this
          # libglvnd rather than its own pinned one. That is the same
          # "`LD_LIBRARY_PATH` leaking into every spawned client" the package
          # itself avoids. Accepted here and only here: glvnd's ABI is stable,
          # this is a dev shell rather than anything shipped, and the
          # alternative is a test suite that cannot run on a NixOS host.
          shellHook = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
            export LIBRARY_PATH="''${LIBRARY_PATH:+$LIBRARY_PATH:}${pkgs.lib.makeLibraryPath (import ./vm/compositor-deps.nix pkgs)}"
            export LD_LIBRARY_PATH="''${LD_LIBRARY_PATH:+$LD_LIBRARY_PATH:}${
              pkgs.lib.makeLibraryPath [ pkgs.libglvnd ]
            }"
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
