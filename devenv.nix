# `devenv shell` -- the same shell as `nix develop`: both read their contents
# from nix/dev-shell.nix (toolchain, compositor libraries, smoke-test tools,
# `soft-egl`, the $XDG_RUNTIME_DIR and locale fallbacks), so the two cannot
# drift apart.
# flake.nix's devShell explains the library paths at length.
#
# `claude.code.enable` is deliberately left off: devenv would then generate
# `.claude/settings.json` and replace the hand-maintained one there.
{ pkgs, lib, ... }:
let
  dev = import ./nix/dev-shell.nix pkgs;
in
{
  packages = dev.toolchain ++ dev.compositorDeps ++ dev.extras;

  env = lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
    LIBRARY_PATH = dev.libraryPath;
    LD_LIBRARY_PATH = dev.ldLibraryPath;
  };

  enterShell = dev.xdgRuntimeDirHook + dev.localeHook;

  enterTest = ''
    cargo --version
    cargo nextest --version
    pkg-config --exists wayland-server xkbcommon pixman-1 libinput
  '';
}
