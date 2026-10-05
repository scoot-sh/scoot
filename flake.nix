{
  # On Linux this flake builds the whole compositor; on macOS the same
  # `scoot` package is client-only (the compositor is cfg'd out, `scoot msg`
  # is the remote-control client) -- so the top level names both, and
  # each package's `meta.description` below comes from its own crate. (The
  # flake-level `description` here has to stay a string literal: the flake
  # loader rejects anything else.)
  description = "scoot: a scrolling-tiling Wayland compositor that runs without a GPU (on macOS, client-only: the scoot msg remote-control client)";

  # Prebuilt binaries of this flake's own packages, pushed by CI
  # (.github/workflows/nix-build.yml) on every merge to main. A flake's
  # `nixConfig` is *not* silently trusted: Nix asks whether to accept it on
  # first use (unless `accept-flake-config` is set), and substituter
  # settings from it apply only to trusted users -- see site/src/content/docs/start/install.md#skip-the-compile-the-binary-cache, which
  # shows the explicit `nix.settings` / `nix.conf` form that needs no
  # prompt and works for untrusted users too.
  nixConfig = {
    extra-substituters = [ "https://scoot-sh.cachix.org" ];
    extra-trusted-public-keys = [
      "scoot-sh.cachix.org-1:QMj7CMw8uqZxrvqqm6SggdxTHz6Q4prt30ydDcXJXCo="
    ];
  };

  # Pinned to the same nixpkgs revision as vm/, so the dev shell and the VM
  # agree on every library and nothing is downloaded twice.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/8ce4ef6cb6f871616146b9fe26d2a5ae594e94fe";

  # The Rust build library: each package below is a cheap `buildPackage`
  # over a shared `buildDepsOnly` compiled-dependencies derivation, so a
  # code-only change stops recompiling Smithay once per package
  # (docs/backlog/resolved/nix-crane-done.md). Pinned by rev like nixpkgs
  # above (this is crane v0.24.0); it takes no inputs of its own, so the
  # lock gains exactly this one node. `devenv.yaml`/vm revs untouched.
  inputs.crane.url = "github:ipetkov/crane/32daa78aa882b7a43c508d1d5a5ad18a7968d731";

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
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
      scootbgDescription =
        (builtins.fromTOML (builtins.readFile ./crates/scootbg/Cargo.toml)).package.description;
      scootbarDescription =
        (builtins.fromTOML (builtins.readFile ./crates/scootbar/Cargo.toml)).package.description;
    in
    {
      # `nix build` / `nix run`, for getting the binaries without a dev shell.
      # `scoot` is the whole compositor (plus the `scoot msg` client, the
      # only client), built `-p scoot` so `$out/bin` carries only the `scoot`
      # binary;
      # `scoot-gpu` is the same binary with the `gpu-scanout` build feature
      # (`--tty --renderer gles` scans out from the GPU instead of reading
      # back; needs OS EGL drivers -- see site/src/content/docs/start/install.md#which-build-do-i-need);
      # `scoot-xwayland` (and `scoot-gpu-xwayland`) add the `xwayland`
      # build feature and nixpkgs' Xwayland on `PATH` (Linux only; X11 apps
      # under `--xwayland` -- see below and site/src/content/docs/scoot/xwayland.md);
      # On Darwin the compositor is cfg'd out of the crate, so `scoot`
      # there is the client (`scoot msg` drives a compositor running
      # elsewhere, e.g. in a VM) -- and `scoot --headless` there exits with
      # a message saying exactly that, so the same package is honest on
      # both (see README's Install section).
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
          # ./crates/{scoot,scootbg,scootbar}/Cargo.toml for
          # the descriptions, ./Cargo.lock for crane's vendor step,
          # ./vm/compositor-deps.nix for buildInputs) resolves against
          # the flake tree, not `src`, so it stays out of the filter.
          # Verified to cover the build: no build.rs outside crates/, no
          # include_str!/include_bytes! of a root-level file, no
          # .cargo/config or toolchain file, and `license.workspace`
          # is a string, not a file read. scootbg's and scootbar's fuzz
          # crates are left out: each is its own workspace (never built
          # here), and its corpus changing must not rebuild the packages.
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              (pkgs.lib.fileset.difference ./crates (
                pkgs.lib.fileset.unions [
                  ./crates/scootbg/fuzz
                  ./crates/scootbar/fuzz
                ]
              ))
            ];
          };

          # The Rust builds below are crane (`buildDepsOnly` for the
          # compiled dependencies, `buildPackage` for each binary), not
          # `buildRustPackage`: one dependency artifact, reused by every
          # package build until the lock or a fork rev changes, instead of
          # one whole-graph compile per package per change.
          craneLib = crane.mkLib pkgs;

          # Shared by every crane derivation below: the fileset source, the
          # lockfile, and the hashes of the two git checkouts. `src`
          # already excludes everything but the workspace (see its
          # comment); `buildDepsOnly` derives dummy manifests from it, so a
          # code-only edit leaves the dependency artifact's hash alone.
          craneCommon = {
            inherit src;
            # Read from the flake tree, not from `src`: the vendor step
            # needs the lock even when it only sees dummy sources.
            cargoLock = ./Cargo.lock;
            # Two dependencies come from git, both scoot-sh forks pinned by
            # rev (docs/forks.md): Smithay (crates/scoot/Cargo.toml, upstream
            # `0ff00983` plus twenty-five commits) and wayland-backend (the root
            # Cargo.toml's `[patch.crates-io]`, the 0.3.17 release plus two
            # commits). A git source carries no crates.io checksum to vendor
            # against, so each checkout hash has to be recorded here, keyed
            # by the source URL as it appears in Cargo.lock (crane's
            # `outputHashes` format, unlike `buildRustPackage`'s
            # name-version keys). The wayland-backend entry covers
            # `wayland-sys` too (one repository, one fetch) and includes the
            # repository's `wayland-protocols` submodule. Bumping either rev
            # changes its hash and fails the build loudly, printing the one
            # it got -- it cannot drift out of sync quietly. Nothing else
            # in Cargo.lock comes from git.
            outputHashes = {
              "git+https://github.com/scoot-sh/smithay?rev=035d447cdf6067f8db58a2964937034a2b9e760b#035d447cdf6067f8db58a2964937034a2b9e760b" =
                "sha256-tyghQjngPRYC2yLjtpF9v/xzUW6ktMJ2gqRWwcryl10=";
              "git+https://github.com/scoot-sh/wayland-rs?rev=70f81e005179463fb4d46ac68045a7f21eb5aff2#70f81e005179463fb4d46ac68045a7f21eb5aff2" =
                "sha256-cANItBOi9o+Jb1+u86thcBgdb6u0u2becgJoMI/v4T8=";
            };
            strictDeps = true;
            # No test run in any crane derivation here, deliberately, for
            # two reasons. This workspace's release profile sets
            # `panic = "abort"`, which cargo ignores for test targets (a
            # test harness has to unwind), so `cargo test --release`
            # rebuilds the whole dependency tree a second time -- and on
            # Linux the tree it would rebuild includes Smithay. On top of
            # that the compositor's tests bind a real wayland socket and so
            # need a writable `$XDG_RUNTIME_DIR`, which the build sandbox
            # has no reason to provide. `nix build` is the path to a
            # binary; `cargo test` in `nix develop` is where the suite runs.
            doCheck = false;
          };

          # The `-lEGL` link injection, shared by every derivation below that
          # links it, so the flag string cannot drift between them. Cargo
          # hashes `RUSTFLAGS` into every unit's fingerprint: a dependency
          # artifact built without these flags can serve no package built
          # with them (and vice versa) -- cargo finds no fingerprint for the
          # differently-flagged units and recompiles the whole graph. So each
          # artifact below carries exactly the flags of the package builds
          # that consume it. (Measured on aarch64-linux: without this parity
          # a code-only `scoot` rebuild recompiled all 129 units.)
          eglRustflags = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux (
            toString (
              map (arg: "-C link-arg=" + arg) [
                "-Wl,--push-state,--no-as-needed"
                "-lEGL"
                "-Wl,--pop-state"
              ]
            )
          );

          # The `-p`/feature selection, shared by each compositor dependency
          # artifact and the package builds over it, for the same reason:
          # cargo unifies features over the selected packages, so `-p scoot`
          # and a workspace-wide selection resolve shared crates (serde and
          # friends) to differently-featured, differently-hashed units, and
          # the package build recompiles that subgraph. Reading byte-identical
          # here on both sides is what makes the reuse real.
          scootArgs =
            features:
            "--locked -p scoot"
            + pkgs.lib.optionalString (features != [ ]) (
              " --features " + pkgs.lib.concatStringsSep "," features
            );

          # The compiled dependencies, as their own store paths -- one per
          # distinct (Cargo feature set, RUSTFLAGS) pair, five in all, and
          # why not fewer: cargo unifies features across the resolve, so
          # Smithay compiled with `backend_gbm` or `xwayland` is a different
          # artifact than Smithay without, and sharing one would recompile
          # Smithay inside each divergent package build. The same holds for
          # the flags above, which is why the compositor's set is separate
          # from the base set: `scoot` (and the three variants) link libEGL,
          # while the client, the wallpaper and the bar link nothing beyond
          # what std links. The base set is shared by `scootbg` and
          # `scootbar` alike: scootbar's feature flags gate only its own
          # modules' code, and its one dependency-bearing flag (`icon-image`
          # → `png`) names a crate the base set already compiles for scootbg,
          # so every `.override` feature set reuses this same artifact --
          # up to the small subgraph whose unified features differ between
          # the workspace-wide selection here and a `-p` selection in the
          # package build (measured: 9--12 crates, seconds, in the serde and
          # wayland-client graphs; Smithay is recompiled in no package).
          # On Darwin the base set below is scoped to the crate that
          # builds there: the wallpaper and the bar refuse non-Linux
          # (`compile_error!` in `scootbg-mem`), so a workspace-wide check
          # would fail where today nothing compositor-shaped is ever
          # compiled.
          mkDeps =
            {
              pname,
              scope,
              withEglLink ? false,
            }:
            craneLib.buildDepsOnly (
              craneCommon
              // {
                # `buildDepsOnly` appends its own `-deps` suffix, so `scoot`
                # below lands as `scoot-deps-0.1.0`.
                inherit pname version;
                nativeBuildInputs = [ pkgs.pkg-config ];
                # The -sys build scripts probe and link these, whatever `-p`
                # scope the artifact is built for.
                buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
                  import ./vm/compositor-deps.nix pkgs
                );
                cargoExtraArgs = scope;
                # Exactly the flags of the consuming package builds (see
                # `eglRustflags` above): present here if and only if present
                # there. Absent entirely when the consumers link nothing --
                # an empty string would do the same work, but absence keeps
                # `nix derivation show` honest about what the build sees.
                env = pkgs.lib.optionalAttrs withEglLink { RUSTFLAGS = eglRustflags; };
              }
            );
          # No `-p` scope on Linux: the whole workspace, so one artifact
          # covers every default package (Darwin scopes to the crates that
          # build there; see the comment above).
          depsBase = mkDeps {
            pname = "scoot-base";
            scope = if pkgs.stdenv.hostPlatform.isDarwin then "--locked -p scoot" else "--locked";
          };
          # The compositor's own set: `-p scoot` (byte-identical to the
          # package build's selection via `scootArgs`) with the link flags.
          # A separate artifact from the base set above, not a scope tweak
          # of it: flags and scope both enter the fingerprint, so sharing
          # would recompile one side's graph inside every build.
          depsScoot = mkDeps {
            pname = "scoot";
            scope = scootArgs [ ];
            withEglLink = true;
          };
          depsGpuScanout = mkDeps {
            pname = "scoot-gpu";
            scope = scootArgs [ "gpu-scanout" ];
            withEglLink = true;
          };
          depsXwayland = mkDeps {
            pname = "scoot-xwayland";
            scope = scootArgs [ "xwayland" ];
            withEglLink = true;
          };
          depsGpuXwayland = mkDeps {
            pname = "scoot-gpu-xwayland";
            scope = scootArgs [
              "gpu-scanout"
              "xwayland"
            ];
            withEglLink = true;
          };

          # Arguments shared by every package build below: sources plus the
          # lock the vendor step needs, with tests off for the reasons
          # above. Each package adds its own `cargoArtifacts` (the
          # dependency set matching its features), its `-p` selection, and
          # the inputs its own link step needs.
          cranePackage = craneCommon // {
            inherit version;
          };

          scoot = craneLib.buildPackage (
            cranePackage
            // {
              pname = "scoot";
              cargoArtifacts = depsScoot;

              # Just this crate, not the whole workspace: `$out/bin` carries
              # only `scoot` (`scoot msg` is part of that binary, not a
              # second one). Without this, the build compiles the whole
              # workspace and ships binaries nothing installs (gh #172).
              # Byte-identical to the `depsScoot` selection via `scootArgs`
              # (see above): any drift recompiles the dependency graph.
              cargoExtraArgs = scootArgs [ ];

              # Read by CI (`ci.yml` asserts the feature pairs) and kept as
              # plain data: crane consumes the features through
              # `cargoExtraArgs` above, while `buildRustPackage` took them as
              # `cargoBuildFeatures` -- the name stays so existing readers
              # keep working.
              passthru.cargoBuildFeatures = [ ];

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
              # EGL drivers; see site/src/content/docs/start/install.md#which-build-do-i-need.
              #
              # Linux-only: these are GNU-ld flags and Apple's ld rejects
              # them (`ld: unknown option: --push-state`), and there is no
              # libEGL to link on Darwin anyway -- the compositor is cfg'd
              # out there, so nothing reaches EGL. (Upstream niri, where this
              # trick comes from, is Linux-only and never hits the question.)
              # Shared with every `buildDepsOnly` artifact that serves a
              # package built with these flags (see `eglRustflags` above).
              env.RUSTFLAGS = eglRustflags;

              # The workspace's release profile sets `strip = true`, and unlike
              # nixpkgs' cargo hook (which used to export
              # `CARGO_PROFILE_RELEASE_STRIP=false` and hand stripping to
              # stdenv, keeping the symbol table: 4,843,776 against 3,619,464
              # measured here) crane leaves the profile alone, so cargo
              # strips fully itself. `stripAllList` stays as the backstop for
              # whatever stdenv's fixup still finds.
              # (No `cargo-auditable` `.dep-v0` section anymore: that came
              # from the old hook's wrapper, not from the profile. Nothing
              # reads it -- no check asserts it -- so it is simply gone.)
              stripAllList = [ "bin" ];

              # `scoot-session`, the greeter-started systemd session launcher
              # (`resources/scoot-session`): beside the binary it launches, so
              # it finds `scoot` next to itself with no path baked in (and the
              # `scoot-xwayland` wrapper's `PATH` append survives, since that
              # is what sits next to it there). A direct store-path reference,
              # not part of `src`'s fileset: the script changes without
              # rebuilding the Rust tree, and the tree rebuilds without
              # re-copying anything but this one file.
              postInstall = ''
                cp ${./resources/scoot-session} $out/bin/scoot-session
                chmod +x $out/bin/scoot-session
              '';

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
            }
          );

          # The wallpaper daemon, its own package (the
          # `scoot` package stays `scoot` only): `-p scootbg` puts just the
          # one binary in `$out/bin`. Pure Rust on the `linux_raw` rustix
          # backend, so, like the client inside `scoot`, nothing to probe or
          # link beyond what std links
          # (docs/scootbg/backlog/resolved/dependencies-done.md §9).
          # Linux only: it is a Wayland client that never runs on a Mac,
          # and does not build there, so no Darwin package is offered.
          scootbg = craneLib.buildPackage (
            cranePackage
            // {
              pname = "scootbg";
              cargoArtifacts = depsBase;
              cargoExtraArgs = "--locked -p scootbg";
              passthru.cargoBuildFeatures = [ ];
              # Same strip backstop and no other inputs as the package above.
              stripAllList = [ "bin" ];
              # `curl` on `PATH` for runtime downloads: `fetch.rs` spawns
              # `curl` by name, so a session `PATH` without it (a minimal
              # NixOS, a container) would fail every URL download loudly
              # (`FetchError::Spawn`). The wrapper appends nixpkgs' curl to
              # `PATH` -- appends, so a `curl` already on the session's
              # `PATH` still wins, and a `scootbg` from anywhere else keeps
              # its plain `PATH` lookup. `makeBinaryWrapper`, not the shell
              # `makeWrapper`, like the Xwayland wrapper below: `scootbg`
              # stays an ELF binary. No new linkage either way -- `curl`
              # is spawned, never linked.
              nativeBuildInputs = [ pkgs.makeBinaryWrapper ];
              postFixup = ''
                wrapProgram $out/bin/scootbg \
                  --suffix PATH : ${pkgs.lib.makeBinPath [ pkgs.curl ]}
              '';
              meta = {
                description = scootbgDescription;
                homepage = "https://github.com/scoot-sh/scoot";
                license = pkgs.lib.licenses.mit;
                mainProgram = "scootbg";
                platforms = pkgs.lib.platforms.linux;
              };
            }
          );

          # The status bar, the same shape as scootbg (Linux only, one
          # binary, nothing linked beyond what std links, no font in its
          # closure), in its own file so that its Cargo features are
          # `callPackage` arguments and a build with other modules is an
          # `.override` (see nix/scootbar.nix). Shares the base dependency
          # artifact (see `mkDeps` above for why every feature set can).
          scootbar = pkgs.callPackage ./nix/scootbar.nix {
            inherit version craneLib;
            inherit (craneCommon)
              src
              cargoLock
              outputHashes
              strictDeps
              doCheck
              ;
            cargoArtifacts = depsBase;
            description = scootbarDescription;
          };

          # One compositor build per Cargo feature set, each over the
          # dependency artifact compiled with that same set (see `mkDeps`).
          # `overrideAttrs` only reaches a finished derivation's own attrs,
          # never the feature selection the dependency artifact was built
          # with, so these are separate `buildPackage` calls through one
          # helper rather than overrides of `scoot` -- same binaries, names
          # and metadata either way. `cargoBuildFeatures` is kept as plain
          # data alongside: crane consumes the features through
          # `cargoExtraArgs`, while `buildRustPackage` took them as
          # `cargoBuildFeatures` -- the name stays so existing readers
          # (`ci.yml`, `nix/tests.nix`) keep working.
          mkScootVariant =
            {
              pname,
              features,
              cargoArtifacts,
              descriptionSuffix,
            }:
            craneLib.buildPackage (
              cranePackage
              // {
                inherit pname cargoArtifacts;
                # Byte-identical to the serving artifact's selection (each
                # variant names its own `deps*` artifact built with the same
                # features through `scootArgs`); see above for why drift
                # costs the whole dependency graph.
                cargoExtraArgs = scootArgs features;
                passthru.cargoBuildFeatures = features;
                nativeBuildInputs = [ pkgs.pkg-config ];
                buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (
                  import ./vm/compositor-deps.nix pkgs
                );
                env.RUSTFLAGS = eglRustflags;
                stripAllList = [ "bin" ];
                postInstall = ''
                  cp ${./resources/scoot-session} $out/bin/scoot-session
                  chmod +x $out/bin/scoot-session
                '';
                meta = {
                  # Same per-system honesty as the base package above:
                  # compositor on Linux, `scoot msg` client on Darwin.
                  description =
                    (
                      if pkgs.stdenv.hostPlatform.isDarwin then
                        "Remote-control client (`scoot msg`) for the scoot scrolling-tiling Wayland compositor"
                      else
                        crateDescription
                    )
                    + descriptionSuffix;
                  homepage = "https://github.com/scoot-sh/scoot";
                  license = pkgs.lib.licenses.mit;
                  mainProgram = "scoot";
                  platforms = pkgs.lib.platforms.linux ++ pkgs.lib.platforms.darwin;
                };
              }
            );

          # The opt-in specs: the base build and the GPU scanout tier, each
          # naming the dependency artifact compiled with its own features.
          # The XWayland packages below derive theirs from these.
          scootSpec = {
            pname = "scoot";
            features = [ ];
            cargoArtifacts = depsScoot;
            descriptionSuffix = "";
          };
          gpuSpec = {
            pname = "scoot-gpu";
            features = [ "gpu-scanout" ];
            cargoArtifacts = depsGpuScanout;
            descriptionSuffix =
              if pkgs.stdenv.hostPlatform.isDarwin then
                " (gpu-scanout build feature: Linux-only, same client binary here)"
              else
                " (gpu-scanout build feature: --tty --renderer gles scans out from the GPU)";
          };
        in
        {
          # `scoot` everywhere: on Linux the compositor, on Darwin the
          # client-only build (`scoot msg` drives a compositor elsewhere).
          default = scoot;
          inherit scoot;
          # The docs site (site/, nix/docs-site.nix): Astro Starlight over
          # pnpm, built offline with hash-pinned dependencies. On every
          # system Node runs on (Linux and Darwin alike): it is a static
          # site, not a compositor binary, so no Linux-only gating applies.
          docs-site = pkgs.callPackage ./nix/docs-site.nix { };
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          inherit scootbg scootbar;
          # scootbar with a nixpkgs font as its default `--font`, so
          # `nix run .#scootbar-demo` shows a clock on a box with no font
          # where the bar looks; a separate output so `scootbar` never
          # carries one (see nix/scootbar-demo.nix).
          scootbar-demo = pkgs.callPackage ./nix/scootbar-demo.nix { inherit scootbar; };
        }
        // {
          # The GPU scanout tier as a package (gh #177): off by default in
          # the build above because `backend_gbm` is a link-time libgbm
          # dependency and GPU-free operation is a fixed decision (see
          # `crates/scoot/Cargo.toml`), opted into here, where linking
          # libgbm is the point. A separate `buildPackage` through
          # `mkScootVariant` over the `gpu-scanout` dependency artifact (not
          # an `overrideAttrs` of `scoot`: the artifact is chosen when the
          # derivation is called, and an override cannot reach back into
          # it); the binary stays named `scoot` (`mainProgram` unchanged),
          # so this is a second build of the same binary with the scanout
          # tier compiled in. `ldd` on the two is the audit: `scoot-gpu`
          # links libgbm, `scoot` does not.
          #
          # Darwin note: `gpu-scanout` enables Smithay's `backend_gbm`,
          # which only compiles on Linux -- but Smithay itself is a
          # Linux-only dependency of this crate, so on Darwin the feature
          # resolves without building anything new and the package stays
          # the same client-shaped binary. Proven by building, not by
          # assuming; if that ever stops holding, gate this to Linux.
          scoot-gpu = mkScootVariant gpuSpec;
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux (
          let
            # The opt-in XWayland tier as packages: the `xwayland` Cargo
            # feature (off in the builds above, like gpu-scanout, because a
            # whole X server is not free -- see site/src/content/docs/scoot/xwayland.md), plus the one
            # runtime dependency it has. Smithay starts the server as
            # `Command::new("Xwayland")`, a `PATH` lookup and nothing else,
            # so a `--tty` login whose `PATH` lacks the binary would be a
            # Wayland-only session (logged loudly, never a failure). The
            # wrapper appends nixpkgs' Xwayland to `PATH` -- appends, so an
            # Xwayland already on the session's `PATH` (NixOS
            # `programs.xwayland.enable`) still wins -- which also means
            # every program the session starts sees that directory at the
            # end of its `PATH`: it holds `Xwayland` and nothing else.
            #
            # `makeBinaryWrapper`, not the shell `makeWrapper`: a small
            # compiled `exec` (`--inherit-argv0`), so `scoot` stays an ELF
            # binary and `scoot msg ...` pays no shell start. The runtime
            # closure grows by Xwayland's own (Mesa for glamor among it:
            # 344 MiB in all, measured on x86_64-linux, of which everything
            # but 11.8 MiB is Xwayland's closure), which is why this is a
            # separate package and never the default. `ldd` of
            # `.scoot-wrapped` is the same audit as the unwrapped builds:
            # the feature links nothing new (no libgbm).
            #
            # Each variant is its spec's build with `xwayland` added --
            # features and the dependency artifact compiled with them both
            # move together (see `mkScootVariant`), so the gpu-scanout
            # variant keeps its own -- with the wrapper on top.
            # `overrideAttrs` only adds the wrapper: it reaches the finished
            # derivation's own attrs, which is all wrapping needs. Linux
            # only: there is no X server, or compositor, on Darwin.
            withXwayland =
              spec: xwaylandDeps:
              (mkScootVariant (
                spec
                // {
                  pname = "${spec.pname}-xwayland";
                  features = spec.features ++ [ "xwayland" ];
                  cargoArtifacts = xwaylandDeps;
                  descriptionSuffix =
                    spec.descriptionSuffix + " (xwayland build feature: --xwayland runs X11 apps; Xwayland on PATH)";
                }
              )).overrideAttrs
                (old: {
                  nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.makeBinaryWrapper ];
                  postFixup = (old.postFixup or "") + ''
                    wrapProgram $out/bin/scoot \
                      --suffix PATH : ${pkgs.lib.makeBinPath [ pkgs.xwayland ]}
                  '';
                });
          in
          {
            scoot-xwayland = withXwayland scootSpec depsXwayland;
            scoot-gpu-xwayland = withXwayland gpuSpec depsGpuXwayland;
          }
        )
      );

      # So `nix run . -- --headless -- foot` and `nix run . -- msg windows`
      # work, `nix run .#scoot-gpu -- --tty -- ...` runs the scanout
      # build and `nix run .#scoot-xwayland -- --headless --xwayland -- xterm`
      # the XWayland one (Linux only), `nix run .#scootbar -- daemon --font
      # F` the bar and `nix run .#scootbar-demo` the bar with a font (Linux
      # only). Each `mainProgram` would resolve
      # without the explicit naming; keeping it explicit rather than implied.
      apps = forEach (
        pkgs:
        {
          default = {
            type = "app";
            program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          };
          scoot-gpu = {
            type = "app";
            program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.scoot-gpu;
          };
        }
        // nixpkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          scoot-xwayland = {
            type = "app";
            program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.scoot-xwayland;
          };
          scootbar = {
            type = "app";
            program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.scootbar;
          };
          scootbar-demo = {
            type = "app";
            program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.scootbar-demo;
          };
        }
      );

      # The contents live in nix/dev-shell.nix, shared with devenv.nix, so
      # `nix develop` and `devenv shell` cannot drift apart. Beyond the
      # toolchain and the compositor's libraries they carry the smoke test's
      # tools, the `soft-egl` wrapper for the GLES tests, and an
      # `$XDG_RUNTIME_DIR` and a UTF-8 `LANG` for boxes that set neither --
      # all in the VM's system closure too, so entering it there fetches
      # nothing new (the `soft-egl` script is a trivial local build).
      devShells = forEach (
        pkgs:
        let
          dev = import ./nix/dev-shell.nix pkgs;
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs =
              dev.toolchain
              ++ dev.extras
              # Not on Linux: the VM's system closure deliberately drops
              # rust-analyzer to keep erofs packing fast, and `nix develop` there
              # should stay a subset of that closure -- nothing new to fetch.
              ++ pkgs.lib.optional pkgs.stdenv.hostPlatform.isDarwin pkgs.rust-analyzer;
            # The compositor's C dependencies exist only on Linux; the core, the
            # IPC crate and the `scoot msg` client build anywhere.
            buildInputs = dev.compositorDeps;

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
              export LIBRARY_PATH="''${LIBRARY_PATH:+$LIBRARY_PATH:}${dev.libraryPath}"
              export LD_LIBRARY_PATH="''${LD_LIBRARY_PATH:+$LD_LIBRARY_PATH:}${dev.ldLibraryPath}"
              ${dev.xdgRuntimeDirHook}
              ${dev.localeHook}
            '';
          };
        }
      );

      # `pkgs.scoot` and (Linux only, where they build)
      # `pkgs.scootbg` and `pkgs.scootbar`: this flake's own builds, the
      # same derivations as `packages`, so an overlay user runs exactly
      # what the flake ships -- scoot and scootbg from one revision, the
      # `apply-config` pair matched -- and builds nothing twice. Not rebuilt against the
      # consumer's nixpkgs (`final.rustPlatform`): that would tie the
      # build to whatever Rust their nixpkgs carries (this workspace needs
      # edition 2024) and split the pinned pair. With it applied, the
      # modules below default `package` and `wallpaper.package` to these,
      # so a direct-module user needs no hand-set package. A system this
      # flake does not build for gets nothing rather than an eval error.
      overlays.default =
        final: prev:
        let
          built = self.packages.${prev.stdenv.hostPlatform.system} or { };
        in
        nixpkgs.lib.getAttrs (builtins.filter (name: built ? ${name}) [
          "scoot"
          "scootbg"
          # Not `scootbar-demo`: a demo to run, not a package to build on
          # (it is scootbar plus a font a system's own font setup provides).
          "scootbar"
        ]) built;

      # `programs.scoot`: a home-manager module (per-user config file,
      # session script hook, portals.conf install) and a NixOS module
      # (system package + opt-in login-screen session entry). Split the
      # way they are because the config file is per-user while session
      # wiring is system-level -- most compositors ship both, thin.
      # Each wrapper below is `mkDefault`s for `package` and
      # `wallpaper.package` over the pure module in `nix/modules/`: the
      # flake's own builds (the same per-system defaults as `packages`)
      # are injected here, whether or not the overlay is applied, and an
      # explicit setting still wins -- except on Darwin on the HM side
      # (see below). See site/src/content/docs/desktop/index.md.
      homeManagerModules =
        let
          hmWrapper =
            { pkgs, ... }:
            let
              built = self.packages.${pkgs.stdenv.hostPlatform.system} or { };
            in
            {
              imports = [ ./nix/modules/home.nix ];
              # The flake's own `scoot` build: on Darwin that package is
              # client-only (`scoot msg` drives a remote session), so the
              # default is honest there too. The macOS use in
              # site/src/content/docs/start/install.md is config management
              # (the config edited here deploys to a Linux box), which a
              # files-only setup also serves -- but the default now installs
              # the client rather than nothing.
              programs.scoot.package = nixpkgs.lib.mkDefault (built.default or null);
              # scootbg is Linux-only: null elsewhere (and on Darwin a
              # `[wallpaper]` renders as written, for the Linux box).
              programs.scoot.wallpaper.package = nixpkgs.lib.mkDefault (built.scootbg or null);
            };
        in
        {
          default = hmWrapper;
          scoot = hmWrapper;
          # `programs.scootbar` (nix/modules/scootbar-home.nix): its own
          # module, not part of `default`, so importing scoot's changes
          # nothing about the bar. The flake's build is `mkDefault`, as
          # above; null where it does not build (Darwin), where enabling
          # the option is then refused by name.
          scootbar =
            { pkgs, ... }:
            {
              imports = [ ./nix/modules/scootbar-home.nix ];
              programs.scootbar.package = nixpkgs.lib.mkDefault (
                (self.packages.${pkgs.stdenv.hostPlatform.system} or { }).scootbar or null
              );
            };
        };

      # Current home-manager spelling (`homeModules.*`); the legacy
      # `homeManagerModules.*` above keeps working for existing
      # consumers -- both names resolve to the same set.
      homeModules = self.homeManagerModules;

      nixosModules =
        let
          osWrapper =
            { pkgs, ... }:
            let
              built = self.packages.${pkgs.stdenv.hostPlatform.system} or { };
            in
            {
              imports = [ ./nix/modules/nixos.nix ];
              programs.scoot.package = nixpkgs.lib.mkDefault (built.default or null);
              programs.scoot.wallpaper.package = nixpkgs.lib.mkDefault (built.scootbg or null);
            };
        in
        {
          default = osWrapper;
          scoot = osWrapper;
          # `programs.scootbar` (nix/modules/scootbar-nixos.nix), as the
          # home-manager one: separate from `default`.
          scootbar =
            { pkgs, ... }:
            {
              imports = [ ./nix/modules/scootbar-nixos.nix ];
              programs.scootbar.package = nixpkgs.lib.mkDefault (
                (self.packages.${pkgs.stdenv.hostPlatform.system} or { }).scootbar or null
              );
            };
        };

      # Hermetic module checks (see nix/tests.nix): standalone
      # evalModules + rendered-file content assertions. Eval-time Nix,
      # so no benchmark applies -- nothing here runs per-event or
      # per-frame; it runs once per `nix flake check`.
      checks = forEach (
        pkgs:
        {
          scoot-modules = pkgs.callPackage ./nix/tests.nix {
            # The flake's own wrappers, overlay and packages, so the checks
            # cover what a flake consumer imports, not just the pure modules;
            # and aarch64-darwin's package set, evaluated (never built) to
            # prove a macOS home-manager config with a `[wallpaper]` table
            # evaluates (the Linux-only idle tools stay null there, refused
            # loudly if the policy is enabled).
            flake = {
              inherit (self)
                overlays
                packages
                homeModules
                homeManagerModules
                nixosModules
                ;
              homeModule = self.homeModules.scoot;
              nixosModule = self.nixosModules.scoot;
            };
            darwinPkgs = nixpkgs.legacyPackages.aarch64-darwin;
          };
        }
        // nixpkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          # The scootbar modules (nix/scootbar-tests.nix): evaluated with and
          # without Stylix, and the rendered file run through the real bar.
          scootbar-modules = pkgs.callPackage ./nix/scootbar-tests.nix {
            inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) scootbar;
            homeModule = self.homeModules.scootbar;
            nixosModule = self.nixosModules.scootbar;
          };
        }
      );

      formatter = forEach (pkgs: pkgs.nixfmt);
    };
}
