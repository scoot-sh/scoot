# The status bar, `packages.<system>.scootbar` (docs/nix.md, "The status
# bar: scootbar"). Its own file, called with `callPackage` from flake.nix,
# so that its Cargo features are arguments and a different build is an
# `.override` (the `scoot-gpu` style `overrideAttrs` only reaches the
# derivation's own attrs, not `buildRustPackage`'s arguments):
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
{
  lib,
  rustPlatform,
  # From flake.nix, shared with the other packages: the one workspace
  # version, the fileset source and the vendored lock.
  version,
  src,
  cargoLock,
  # `crates/scootbar/Cargo.toml`'s own description, read by flake.nix so
  # it cannot drift from the crate.
  description,
  # The Cargo features, as `buildRustPackage` takes them.
  buildNoDefaultFeatures ? false,
  buildFeatures ? [ ],
}:

rustPlatform.buildRustPackage {
  pname = "scootbar";
  inherit
    version
    src
    cargoLock
    buildNoDefaultFeatures
    buildFeatures
    ;

  cargoBuildFlags = [
    "-p"
    "scootbar"
  ];

  # Same profile-put-back and no-test reasons as the packages in flake.nix:
  # the release profile asks for a full strip, which nixpkgs' cargo hook
  # hands to stdenv (which alone keeps the symbol table), and a test run
  # would rebuild the tree without `panic = "abort"`; CI's scootbar jobs
  # run the suite.
  stripAllList = [ "bin" ];
  doCheck = false;

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
