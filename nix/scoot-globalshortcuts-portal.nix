# The GlobalShortcuts portal backend,
# `packages.<system>.scoot-globalshortcuts-portal`
# (dev/design/global-shortcuts.md).
# Its own file, called with `callPackage` from flake.nix, so that its
# Cargo features are arguments and a different build is an `.override`.
# The crate names no features today (the backend is one build); the
# override plumbing stays so a future feature needs no Nix change.
#
# Like scootnotify: `-p scoot-globalshortcuts-portal` puts just the one
# binary in `$out/bin`, and the daemon is pure Rust on the `linux_raw`
# rustix backend with no C beyond what std links, so there are no
# `buildInputs` to give it.
#
# A crane `buildPackage` over the workspace's shared base dependency
# artifact (every override reuses it: the flags below gate only the
# backend's own code -- see flake.nix's `mkDeps`). `cargoBuildNoDefaultFeatures`/
# `cargoBuildFeatures` stay on the derivation as plain data under those
# names (nix tests and the workflow's override check read them); crane
# itself consumes the selection through `cargoExtraArgs`.
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
  # `crates/scoot-globalshortcuts-portal/Cargo.toml`'s own description,
  # read by flake.nix so it cannot drift from the crate.
  description,
  # The Cargo features, as `.override` takes them (`buildFeatures` adds to
  # the defaults unless `buildNoDefaultFeatures` turns them off -- the same
  # semantics `buildRustPackage` gave these names).
  buildNoDefaultFeatures ? false,
  buildFeatures ? [ ],
}:

craneLib.buildPackage {
  pname = "scoot-globalshortcuts-portal";
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
    "--locked -p scoot-globalshortcuts-portal"
    + lib.optionalString buildNoDefaultFeatures " --no-default-features"
    + lib.optionalString (buildFeatures != [ ]) (
      " --features " + lib.concatStringsSep "," buildFeatures
    );

  passthru = {
    cargoBuildNoDefaultFeatures = buildNoDefaultFeatures;
    cargoBuildFeatures = buildFeatures;
  };

  # Same strip backstop and no-test reasons as the packages in flake.nix:
  # the release profile asks for a full strip, which crane leaves to cargo
  # itself, and a test run would rebuild the tree without
  # `panic = "abort"`; CI's portal job runs the suite.
  stripAllList = [ "bin" ];

  meta = {
    inherit description;
    homepage = "https://github.com/scoot-sh/scoot";
    license = lib.licenses.mit;
    mainProgram = "scoot-globalshortcuts-portal";
    # A session-bus service that never runs on a Mac, and does not build
    # there, so no other package is offered.
    platforms = lib.platforms.linux;
  };
}
