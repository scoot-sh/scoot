# The dev shell's contents, as one list shared by `nix develop` (flake.nix's
# devShell) and `devenv shell` (devenv.nix), so the two cannot drift apart --
# the same reason vm/compositor-deps.nix exists. Each consumer keeps its own
# explanation of *why* the library paths are what they are (flake.nix has
# the long form); this file only says *what*.
#
# Everything Linux-only here is also in the dev VM's system closure
# (vm/configuration.nix), so entering the shell on the VM fetches nothing
# new beyond a local build of the tiny `soft-egl` script.
pkgs:
let
  inherit (pkgs) lib;
  isLinux = pkgs.stdenv.hostPlatform.isLinux;
in
rec {
  toolchain = [
    pkgs.rustc
    pkgs.cargo
    pkgs.clippy
    pkgs.rustfmt
    # Part of the documented verification set (see `CLAUDE.md`).
    pkgs.cargo-nextest
    pkgs.pkg-config
  ];

  # The compositor's C dependencies exist only on Linux; the core, the IPC
  # crate and the `scootctl` client build anywhere.
  compositorDeps = lib.optionals isLinux (import ../vm/compositor-deps.nix pkgs);

  # What scripts/smoke-test.sh drives: `foot` as the client (without it the
  # script cannot run at all), `jq` for IPC replies, ImageMagick to sample
  # screenshot pixels, `wayland-info` for the dmabuf check. The same four CI
  # hands its smoke step through `nix shell`.
  #
  # Wrapped as one package holding only `bin/` symlinks, not listed bare:
  # mkShell runs every input through `chooseDevOutputs`, so a bare
  # `jq`/`imagemagick` brings its `-dev` output and everything that
  # propagates -- about fifty `-dev` outputs (openssl, curl, zlib,
  # freetype, ...) on PKG_CONFIG_PATH and the compiler's search paths, where
  # a future `-sys` crate would silently link against a smoke-test leftover.
  # (`lib.getBin` alone is not enough: imagemagick has no `bin` output, so
  # its `lib/` would still land on NIX_LDFLAGS.) Only the executables are
  # wanted, and this adds nothing but PATH.
  smokeTools = lib.optional isLinux (
    pkgs.runCommandLocal "scoot-smoke-tools" { } ''
      mkdir -p $out/bin
      for tool in ${
        lib.concatMapStringsSep " " (p: "${lib.getBin p}/bin") [
          pkgs.foot
          pkgs.jq
          pkgs.imagemagick
          pkgs.wayland-utils
        ]
      }; do
        ln -s "$tool"/* $out/bin/
      done
    ''
  );

  # `soft-egl <cmd>` runs one command against Mesa's software EGL, the way
  # CI's test steps do ("Resolve a software EGL" in .github/workflows/ci.yml):
  # the GLES tests fail rather than skip without a loadable EGL device.
  # Scoped to the one command and never exported shell-wide, for CI's
  # reasons -- a smoke run must not be handed Mesa it would not otherwise
  # have, and nixpkgs' Mesa must not shadow a host's real driver.
  softEgl = pkgs.writeShellScriptBin "soft-egl" ''
    LD_LIBRARY_PATH="''${LIBRARY_PATH:+$LIBRARY_PATH:}${pkgs.mesa}/lib" \
    __EGL_VENDOR_LIBRARY_DIRS="${pkgs.mesa}/share/glvnd/egl_vendor.d" \
      exec "$@"
  '';

  extras = smokeTools ++ lib.optional isLinux softEgl;

  # Link time: a few -sys crates link with a bare -lfoo.
  libraryPath = lib.makeLibraryPath compositorDeps;
  # Run time: Smithay dlopens libEGL. libglvnd only, never Mesa.
  ldLibraryPath = lib.makeLibraryPath [ pkgs.libglvnd ];

  # The compositor, and the tests that bind a real wayland socket, need a
  # writable $XDG_RUNTIME_DIR. A login session provides one; a container or
  # CI shell often doesn't. Only fill the gap, never override a real one.
  # The path must be stable across shell entries -- `scoot msg` in a second
  # `devenv shell` finds the compositor's socket through it -- and owned by
  # us, not a symlink, so another user cannot pre-create it and receive the
  # compositor's sockets. /run/user/<uid> first (scripts/smoke-test.sh
  # defaults there), then /tmp/scoot-runtime-<uid> (plain /tmp, not
  # $TMPDIR, which `nix develop` deletes on exit), and a one-off mktemp dir
  # only when both are taken by someone else.
  xdgRuntimeDirHook = lib.optionalString isLinux ''
    if [ -z "''${XDG_RUNTIME_DIR:-}" ] || [ ! -w "$XDG_RUNTIME_DIR" ]; then
      XDG_RUNTIME_DIR=
      for candidate in "/run/user/$(id -u)" "/tmp/scoot-runtime-$(id -u)"; do
        mkdir -p -m 0700 "$candidate" 2>/dev/null
        if [ ! -L "$candidate" ] && [ -O "$candidate" ] && [ -w "$candidate" ]; then
          XDG_RUNTIME_DIR=$candidate
          break
        fi
      done
      if [ -z "$XDG_RUNTIME_DIR" ]; then
        XDG_RUNTIME_DIR=$(mktemp -d /tmp/scoot-runtime.XXXXXX)
      fi
      export XDG_RUNTIME_DIR
    fi
  '';
}
