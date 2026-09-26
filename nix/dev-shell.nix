# The dev shell's contents, as one list shared by `nix develop` (flake.nix's
# devShell) and `devenv shell` (devenv.nix), so the two cannot drift apart --
# the same reason vm/compositor-deps.nix exists. Each consumer keeps its own
# explanation of *why* the library paths are what they are (flake.nix has
# the long form); this file only says *what*.
#
# Everything Linux-only here is also in the dev VM's system closure
# (vm/configuration.nix), so the Linux shell stays a subset of it and
# entering it on the VM fetches nothing new.
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
  smokeTools = lib.optionals isLinux [
    pkgs.foot
    pkgs.jq
    pkgs.imagemagick
    pkgs.wayland-utils
  ];

  # `soft-egl <cmd>` runs one command against Mesa's software EGL, the way
  # CI's test steps do ("Resolve a software EGL" in .github/workflows/ci.yml):
  # the GLES tests fail rather than skip without a loadable EGL device.
  # Scoped to the one command and never exported shell-wide, for CI's
  # reasons -- the smoke test must keep proving scoot runs with no EGL at
  # all, and nixpkgs' Mesa must not shadow a host's real driver.
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
  # CI shell often doesn't. Only fill the gap, never override a real one:
  # /run/user/<uid> first (scripts/smoke-test.sh defaults its sockets
  # there), a private /tmp dir where /run isn't writable.
  xdgRuntimeDirHook = lib.optionalString isLinux ''
    if [ -z "''${XDG_RUNTIME_DIR:-}" ] || [ ! -w "$XDG_RUNTIME_DIR" ]; then
      XDG_RUNTIME_DIR="/run/user/$(id -u)"
      if ! mkdir -p -m 0700 "$XDG_RUNTIME_DIR" 2>/dev/null || [ ! -w "$XDG_RUNTIME_DIR" ]; then
        XDG_RUNTIME_DIR="/tmp/scoot-runtime-$(id -u)"
        mkdir -p -m 0700 "$XDG_RUNTIME_DIR"
      fi
      export XDG_RUNTIME_DIR
    fi
  '';
}
