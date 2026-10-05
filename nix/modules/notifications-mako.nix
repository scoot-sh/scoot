# The notification daemon's package: nixpkgs' mako without the GTK
# stack. `wrapGAppsHook3` (in mako's `nativeBuildInputs` at the pinned
# rev) pulls gtk+3 plus tinysparql, cups, at-spi2-core, avahi and
# librsvg into the closure -- about 210 MiB over what the desktop
# profile already has (measured against swaylock's closure, same
# method as docs/nix.md "Why mako"). mako itself needs none of it: no
# GSettings schemas of its own, no toolkit. This override drops the
# hook and wraps only what mako uses, the way dunst's own package
# wraps its pixbuf loaders:
#
# - `GDK_PIXBUF_MODULE_FILE`: the icon loaders (png, svg through
#   librsvg, ...), built explicitly with the cache builder so the
#   wrapper never depends on the build-time environment variable.
# - `XDG_DATA_DIRS`: hicolor-icon-theme, so themed icon names resolve.
# - `PATH`: busctl/jq/bash, exactly what upstream's `preFixup` adds.
#
# Result at the pinned rev: 183.4 MiB closure (was 357.4), 53.3 MiB
# over swaylock's closure (was 210.4), zero references to gtk+3,
# tinysparql, cups, at-spi2-core or avahi (pinned in `nix/tests.nix`:
# the closure check fails if any of those names reappears). X client
# libraries (libX11, libxcb, ...) still ride along through
# cairo/pango -- shared with the lock child, not new -- and mako
# still cannot run on X11 at all (it has no X11 backend; that, not
# the closure, is the distinction from dunst, which compiles both).
#
# A package expression, not a module: takes `pkgs`, returns the
# derivation. Both `package` defaults (home-manager and NixOS) and
# `nix/tests.nix` import it, so the three never drift apart. Never
# imported off Linux: `pkgs.mako` refuses evaluation there (its
# attribute exists on Darwin but fails when forced), so callers guard
# with `stdenv.hostPlatform.isLinux` first and this file has no such
# guard of its own.
{ pkgs }:
let
  lib = pkgs.lib;
  pixbufCache = pkgs.gnome._gdkPixbufCacheBuilder_DO_NOT_USE {
    extraLoaders = [ pkgs.librsvg ];
  };
in
pkgs.mako.overrideAttrs (old: {
  nativeBuildInputs =
    builtins.filter (i: (i.name or "") != "wrap-gapps-hook") old.nativeBuildInputs
    ++ [ pkgs.makeWrapper ];
  # librsvg for the svg icon loader (its cache entry is combined
  # with gdk-pixbuf's own loaders by the builder above).
  buildInputs = old.buildInputs ++ [ pkgs.librsvg ];
  # Through `env.` so the value is set for every phase including
  # `preFixup`: relying on the gdk-pixbuf setup hook's own variable
  # does not work here (measured: it is empty at `preFixup` once the
  # hook is gone, where the stock build sees librsvg's combined
  # cache), so build the cache explicitly instead.
  env.GDK_PIXBUF_MODULE_FILE = "${pixbufCache}";
  preFixup = ''
    wrapProgram $out/bin/mako \
      --set GDK_PIXBUF_MODULE_FILE "$GDK_PIXBUF_MODULE_FILE" \
      --prefix PATH : "${
        lib.makeBinPath [
          pkgs.systemdMinimal # for busctl
          pkgs.jq
          pkgs.bash
        ]
      }" \
      --prefix XDG_DATA_DIRS : "${pkgs.hicolor-icon-theme}/share"
    wrapProgram $out/bin/makoctl \
      --prefix PATH : "${
        lib.makeBinPath [
          pkgs.systemdMinimal
          pkgs.jq
          pkgs.bash
        ]
      }"
  '';
})
