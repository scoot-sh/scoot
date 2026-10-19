# The launcher, `packages.<system>.scootlaunch` (dev/design/scootlaunch.md).
# Its own file, called with `callPackage` from flake.nix, so that its
# Cargo features are arguments and a different build is an `.override`:
#
#   scootlaunch.override { buildNoDefaultFeatures = true; }    # no svg
#   scootlaunch.override {
#     buildNoDefaultFeatures = true;
#     buildFeatures = [ "svg" ];
#   }
#
# The feature names are the crate's own (`crates/scootlaunch/Cargo.toml`:
# one optional dependency, `svg`, on by default).
#
# Like scootbar: `-p scootlaunch` puts just the one binary in `$out/bin`,
# and the launcher is pure Rust on the `linux_raw` rustix backend with no
# C beyond what std links plus libxkbcommon (the panel's seat keyboard;
# see the link gate in `.github/workflows/ci.yml`, which refuses any
# other library), so there are no `buildInputs` to give it.
#
# A crane `buildPackage` over the workspace's shared base dependency
# artifact (every override reuses it: the flags below gate only the
# launcher's own `svg` module's code -- see flake.nix's `mkDeps`).
# `cargoBuildNoDefaultFeatures`/`cargoBuildFeatures` stay on the
# derivation as plain data under those names (nix tests and the
# workflow's override check read them); crane itself consumes the
# selection through `cargoExtraArgs`.
{
  lib,
  pkgs,
  # From flake.nix, shared with the other packages: that package's own
  # version, the fileset source, the lockfile and the git-checkout hashes,
  # the strict-deps/test settings, and the dependency artifact this builds
  # over.
  version,
  src,
  cargoLock,
  outputHashes,
  strictDeps,
  doCheck,
  craneLib,
  cargoArtifacts,
  # `crates/scootlaunch/Cargo.toml`'s own description, read by flake.nix
  # so it cannot drift from the crate.
  description,
  # The Cargo features, as `.override` takes them (`buildFeatures` adds to
  # the defaults unless `buildNoDefaultFeatures` turns them off -- the
  # same semantics `buildRustPackage` gave these names).
  buildNoDefaultFeatures ? false,
  buildFeatures ? [ ],
}:

craneLib.buildPackage {
  pname = "scootlaunch";
  inherit
    version
    src
    cargoLock
    outputHashes
    strictDeps
    doCheck
    cargoArtifacts
    ;

  cargoExtraArgs =
    "--locked -p scootlaunch"
    + lib.optionalString buildNoDefaultFeatures " --no-default-features"
    + lib.optionalString (buildFeatures != [ ]) (
      " --features " + lib.concatStringsSep "," buildFeatures
    );

  passthru = {
    cargoBuildNoDefaultFeatures = buildNoDefaultFeatures;
    cargoBuildFeatures = buildFeatures;
  };

  # The one C library past std's the panel links (the seat keyboard,
  # through xkbcommon-sys): on the link path the way the compositor's
  # own inputs are, with pkg-config for the -sys build script's probe.
  # Linux-only, like the package itself.
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.libxkbcommon ];

  # Same strip backstop and no-test reasons as the packages in flake.nix:
  # the release profile asks for a full strip, which crane leaves to cargo
  # itself, and a test run would rebuild the tree without
  # `panic = "abort"`; CI's scootlaunch job runs the suite.
  stripAllList = [ "bin" ];

  meta = {
    inherit description;
    homepage = "https://github.com/scoot-sh/scoot";
    license = lib.licenses.mit;
    mainProgram = "scootlaunch";
    # A Wayland client that never runs on a Mac, and does not build
    # there, so no other package is offered.
    platforms = lib.platforms.linux;
  };
}
