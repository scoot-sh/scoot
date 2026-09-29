# `packages.<system>.scootbar-demo`: scootbar with a font, so that
# `nix run .#scootbar-demo` shows a clock on a NixOS box with no font in
# any of the places the bare binary looks. The bare `scootbar` never
# carries one (its installed closure is a measured row, docs/scootbar/
# backlog/lightest.md), and the modules do not use this.
#
# The font is DejaVu Sans alone (`dejavu_fonts.minimal`, one 742 KiB
# file), the face the bar's well-known list looks for first. It is a
# default, not a fixed setting:
#
#   scootbar-demo                          scootbar daemon --font DEJAVU
#   scootbar-demo daemon [FLAGS]           the same, with FLAGS
#   scootbar-demo daemon --font F [FLAGS]  as given: F wins
#   scootbar-demo daemon --help, --help, --version, anything else: as given
#
# A script rather than `makeBinaryWrapper --add-flags`: the flag belongs
# after `daemon`, only when `--font` is not already there (the bar refuses
# a flag given twice), and never before `--help`, which `daemon` takes only
# as its first argument. The shell costs about 1.7 ms, once, at start
# (`--version`: 2.3 ms bare, 4.0 ms through this, mean of 100 runs), on a
# demo; the bare package is the one measured.
{
  lib,
  writeTextFile,
  runtimeShell,
  shellcheck-minimal,
  dejavu_fonts,
  scootbar,
}:

let
  font = "${dejavu_fonts.minimal}/share/fonts/truetype/DejaVuSans.ttf";
in
writeTextFile {
  name = "scootbar-demo-${scootbar.version}";
  destination = "/bin/scootbar";
  executable = true;
  text = ''
    #!${runtimeShell}
    # scootbar with ${font}
    # as its default --font (see nix/scootbar-demo.nix in scoot's flake).
    bar=${lib.getExe scootbar}
    font=${font}

    if [ "$#" -eq 0 ]; then
      exec "$bar" daemon --font "$font"
    fi
    if [ "$1" != daemon ]; then
      exec "$bar" "$@"
    fi
    case "''${2-}" in
      --help | -h) exec "$bar" "$@" ;;
    esac

    # Every daemon flag takes a value, as `--flag VALUE` or `--flag=VALUE`,
    # so a value is skipped rather than read as a flag: `--clock-format
    # --font` is not a font given.
    skip=
    for arg in "''${@:2}"; do
      if [ -n "$skip" ]; then
        skip=
        continue
      fi
      case $arg in
        --font | --font=*) exec "$bar" "$@" ;;
        --*=*) ;;
        --*) skip=1 ;;
      esac
    done
    exec "$bar" daemon --font "$font" "''${@:2}"
  '';
  checkPhase = ''
    ${lib.getExe shellcheck-minimal} "$target"
    test -f ${font}
  '';
  meta = {
    description = "${scootbar.meta.description}, with DejaVu Sans as its default font (a demo)";
    inherit (scootbar.meta) homepage license platforms;
    mainProgram = "scootbar";
  };
}
