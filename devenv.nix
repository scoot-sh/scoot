# `devenv shell` -- the same toolchain and libraries as `nix develop` (see
# the devShell in flake.nix), plus the tools scripts/smoke-test.sh drives, so
# the whole verification set in CLAUDE.md runs from one shell. The C library
# list is shared with the flake and the VM through vm/compositor-deps.nix, so
# the three cannot drift apart.
#
# `claude.code.enable` is deliberately left off: devenv would then generate
# `.claude/settings.json` and replace the hand-maintained one there.
{ pkgs, lib, ... }:
let
  isLinux = pkgs.stdenv.hostPlatform.isLinux;
  compositorDeps = lib.optionals isLinux (import ./vm/compositor-deps.nix pkgs);
in
{
  packages = [
    pkgs.rustc
    pkgs.cargo
    pkgs.clippy
    pkgs.rustfmt
    pkgs.cargo-nextest
    pkgs.pkg-config
  ]
  ++ compositorDeps
  # What scripts/smoke-test.sh launches or probes inside the compositor.
  # Each one it can't find only skips that check, but without foot the
  # script cannot run at all.
  ++ lib.optionals isLinux [
    pkgs.foot
    pkgs.wayland-utils # wayland-info
    pkgs.imagemagick
    pkgs.xwayland
    pkgs.xeyes
  ];

  # Same two exports as the flake devShell's shellHook, for the same reasons
  # (explained at length there): a few -sys crates link with a bare -lfoo that
  # needs LIBRARY_PATH, and Smithay dlopens libEGL at run time, so libglvnd
  # (only libglvnd, never Mesa) goes on LD_LIBRARY_PATH.
  env = lib.optionalAttrs isLinux {
    LIBRARY_PATH = lib.makeLibraryPath compositorDeps;
    LD_LIBRARY_PATH = lib.makeLibraryPath [ pkgs.libglvnd ];
  };

  # `soft-egl <cmd>` runs one command against Mesa's software EGL, the way CI
  # runs the test suite (see "Resolve a software EGL" in
  # .github/workflows/ci.yml): the GLES tests fail rather than skip without a
  # loadable EGL device. Scoped to the one command, never exported shell-wide,
  # for the reasons CI gives -- the smoke test must keep proving scoot runs
  # with no EGL at all, and Mesa must not shadow a host's real driver.
  #   soft-egl cargo nextest run --workspace
  scripts.soft-egl = lib.mkIf isLinux {
    description = "Run a command against Mesa's software EGL (for the GLES tests)";
    exec = ''
      LD_LIBRARY_PATH="$LIBRARY_PATH:${pkgs.mesa}/lib" \
      __EGL_VENDOR_LIBRARY_DIRS="${pkgs.mesa}/share/glvnd/egl_vendor.d" \
        exec "$@"
    '';
  };

  # The compositor (and the tests that bind a real wayland socket) need a
  # writable $XDG_RUNTIME_DIR. A login session provides one; a container or
  # CI shell often doesn't. Only fill the gap -- never override a real one.
  # The conventional /run/user/<uid> first, since scripts/smoke-test.sh
  # defaults its socket paths there; a private /tmp dir where /run isn't
  # writable.
  enterShell = lib.optionalString isLinux ''
    if [ -z "''${XDG_RUNTIME_DIR:-}" ] || [ ! -w "$XDG_RUNTIME_DIR" ]; then
      XDG_RUNTIME_DIR="/run/user/$(id -u)"
      if ! mkdir -p -m 0700 "$XDG_RUNTIME_DIR" 2>/dev/null || [ ! -w "$XDG_RUNTIME_DIR" ]; then
        XDG_RUNTIME_DIR="/tmp/scoot-runtime-$(id -u)"
        mkdir -p -m 0700 "$XDG_RUNTIME_DIR"
      fi
      export XDG_RUNTIME_DIR
    fi
  '';

  enterTest = ''
    cargo --version
    cargo nextest --version
    pkg-config --exists wayland-server xkbcommon pixman-1 libinput
  '';
}
