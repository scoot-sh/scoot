# Seeds the live look link for the daemon leaves (`look-leaves-daemons`):
# mako's `include=` and wob's `-c` point at `<state>/look/…`, and both
# daemons refuse to start when that link dangles (checked against mako
# 1.11.0 `config.c` and wob 0.16 `src/main.c`: a missing file is
# `EXIT_FAILURE`, not a warning). No user has a recorded choice before
# the first apply, so without this a fresh install's first login starts
# with dead notifications and no OSD.
#
# The seed runs `scoot look apply` for the declared default look once,
# when no choice is recorded yet (that is `sync` rule 1; `look-nix-catalog`
# owns the full `sync` and supersedes this stopgap). The catalog is the
# flake's own `looks/` sources (`SCOOT_LOOKS_DIR`, which the catalog
# search reads as shipped looks), so this needs no catalog generation.
# The apply skips every reload with a note (nothing runs at activation),
# renders without installed fonts when fontconfig is absent (a note, not
# a refusal), and leaves the session bus alone when it cannot reach it.
# Gated to a declared look with a real binary: files-only setups and
# unthemed machines have no link pointing anywhere, so they seed nothing.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  look = cfg.desktop.look or null;
  # `or null`: a standalone leaf import without the profile entry leaves
  # `package` undefined, and a files-only setup sets it to null; neither
  # seeds anything.
  package = cfg.package or null;
  # `lib.hm.dag` exists under real home-manager only: the eval stubs in
  # `nix/tests.nix` use plain nixpkgs lib, so fall back to a plain
  # after-entry there (activation never runs in an eval).
  entryAfter =
    lib.hm.dag.entryAfter or (deps: text: {
      data = text;
      after = deps;
    });
in
{
  home.activation.scootLookSeed = lib.mkIf (look != null && package != null) (
    entryAfter [ "linkGeneration" ] ''
      state="''${XDG_STATE_HOME:-$HOME/.local/state}/scoot"
      if [ ! -e "$state/look.json" ]; then
        echo "seeding the live look choice (${look}): first apply runs now" >&2
        SCOOT_LOOKS_DIR=${../../looks} ${lib.getExe package} look apply "${look}" >&2 || \
          echo "warning: the look seed failed (run \`scoot look apply ${look}\` by hand before starting mako or wob)" >&2
      fi
    ''
  );
}
