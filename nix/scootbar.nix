# The status bar, `packages.<system>.scootbar` (docs/nix.md, "The status
# bar: scootbar"). Its own file, called with `callPackage` from flake.nix,
# so that its Cargo features are arguments and a different build is an
# `.override`:
#
#   scootbar.override { buildNoDefaultFeatures = true; }    # no modules
#   scootbar.override {
#     buildNoDefaultFeatures = true;
#     buildFeatures = [ "clock" ];
#   }
#
# The feature names are the crate's own (`crates/scootbar/Cargo.toml`,
# one per module, `clock` the default).
#
# Like scootbg: `-p scootbar` puts just the one binary in `$out/bin`, and
# the bar is pure Rust on the `linux_raw` rustix backend with no C beyond
# what std links (no libwayland, no libEGL, no fontconfig), so there are no
# `buildInputs` to give it. **No font is in its closure**: the binary takes
# one from `--font` or a short list of well-known files and otherwise
# refuses to start saying how to give one (docs/scootbar/cli.md#fonts).
# `scootbar-demo` (./scootbar-demo.nix) is the build that carries one.
#
# A crane `buildPackage` over the workspace's shared dependency artifact
# (every override reuses it: the flags below gate only the bar's own
# modules' code, and the one dependency-bearing flag, `icon-image`'s
# `png`, is already compiled for scoot and scootbg -- see flake.nix's
# `mkDeps`). `cargoBuildNoDefaultFeatures`/`cargoBuildFeatures` stay on the
# derivation as plain data under those names (`nix/tests.nix` and the
# workflow's override check read them); crane itself consumes the selection
# through `cargoExtraArgs`.
{
  lib,
  # From flake.nix, shared with the other packages: the one workspace
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
  # `crates/scootbar/Cargo.toml`'s own description, read by flake.nix so
  # it cannot drift from the crate.
  description,
  # The Cargo features, as `.override` takes them (`buildFeatures` adds to
  # the defaults unless `buildNoDefaultFeatures` turns them off -- the same
  # semantics `buildRustPackage` gave these names).
  buildNoDefaultFeatures ? false,
  buildFeatures ? [ ],
}:

craneLib.buildPackage {
  pname = "scootbar";
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
    "--locked -p scootbar"
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
  # `panic = "abort"`; CI's scootbar jobs run the suite.
  stripAllList = [ "bin" ];

  meta = {
    inherit description;
    homepage = "https://github.com/scoot-sh/scoot";
    license = lib.licenses.mit;
    mainProgram = "scootbar";
    # A Wayland client that never runs on a Mac, and does not build
    # there, so no other package is offered.
    platforms = lib.platforms.linux;
  };
}
