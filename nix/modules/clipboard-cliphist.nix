# The clipboard manager's package: nixpkgs' cliphist without the contrib
# picker scripts. Those scripts (`contrib/cliphist-{fuzzel-img,fzf,...}`)
# embed absolute store paths of fuzzel, fzf, chafa and wofi (gtk+3 with its
# tinysparql/cups/at-spi2 train), which lands ~208 MiB of marginal closure
# on the profile for pickers the profile never runs: the history picker is
# the profile's own `scoot-clipboard-pick` script (cliphist through fuzzel's
# dmenu mode, themed by the look -- see `keys-home.nix`), so the contrib
# scripts are dead weight, not choice. This override drops them from
# `postInstall`, keeping the `cliphist` binary itself (bbolt history db,
# bounded size, dedupe, previews, images byte-for-byte).
#
# Result at the pinned rev: the binary plus its Go runtime only (a few MiB
# marginal over the profile, pinned in `nix/tests.nix` the way the lean
# mako's no-GTK-stack check is: any of those names reappearing fails
# loudly). `wl-clipboard` stays stock nixpkgs (it is already lean: C,
# wayland-client only).
#
# A package expression, not a module: takes `pkgs`, returns the
# derivation. Both `package` defaults (home-manager and NixOS) and
# `nix/tests.nix` import it, so the three never drift apart. Never
# imported off Linux: like `pkgs.mako`, `pkgs.cliphist` is Linux-only
# (`meta.platforms = linux`), so callers guard with
# `stdenv.hostPlatform.isLinux` first and this file has no such guard of
# its own.
{ pkgs }:
pkgs.cliphist.overrideAttrs (old: {
  # Upstream's `postInstall` copies the contrib pickers beside the binary;
  # keep the binary, skip the pickers (see above). `postFixup` (the Go
  # wrapper) still runs: overriding `postInstall` leaves the phase list
  # alone.
  postInstall = "";
})
