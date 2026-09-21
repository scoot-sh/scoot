# Hermetic checks for the scoot NixOS/home-manager modules, wired into
# `nix flake check` as `checks.<system>.scoot-modules`. No home-manager or
# NixOS installation needed: each module is evaluated standalone with stub
# options standing in for what those module systems provide
# (`xdg.configFile`/`home.packages` on the HM side,
# `environment.systemPackages`/`services.displayManager.*` on the NixOS
# side), and the rendered files are validated with python's tomllib.
#
# What this pins, per edge case:
# - empty settings render a valid minimal file (parses to `{}` -- the
#   compositor runs it as pure defaults, proven live, not just here);
# - `[binds]` keys needing TOML quoting round-trip byte-equal through
#   nix -> TOML -> python (then live through the real loader on Linux);
# - `enable = false` manages no files and installs nothing;
# - the session script is written beside the config: a relocated
#   `configFile` carries its script with it (nothing left at the
#   default path), the default keeps `scoot/session.sh`;
# - the session entry is additive (default session untouched) and carries
#   the `providedSessions` nixpkgs requires of every session package;
# - `session.command` renders verbatim into `Exec=` (bare `--tty` default
#   byte-identical, `-- COMMAND` append, wrapper-script path, quoting
#   with spaces/quotes/pipes intact), stays inert with the entry off,
#   and refuses empty / package-less combinations at eval;
# - the two settings failure modes behave as documented (see below).
{
  lib,
  pkgs,
  runCommand,
  python3,
}:

let
  tomlFormat = pkgs.formats.toml { };

  # `lib.evalModules` enforces `config.assertions` itself, but the
  # option declaration lives in the NixOS/home-manager cores, so a
  # standalone evaluation must declare it (same shape as
  # `nixos/modules/misc/assertions.nix`).
  baseStubs = {
    options.assertions = lib.mkOption {
      type = lib.types.listOf (
        lib.types.submodule {
          options = {
            assertion = lib.mkOption { type = lib.types.bool; };
            message = lib.mkOption { type = lib.types.str; };
          };
        }
      );
      default = [ ];
    };
    options.warnings = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
    };
  };

  # Stand-ins for the options the real module systems own. Kept minimal
  # on purpose: they assert OUR modules' wiring, not home-manager's or
  # NixOS's (whose real definitions must agree -- if they ever reject
  # what we set, that surfaces the moment a user evaluates for real).
  homeStubs = {
    options.home.packages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.xdg.configFile = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options = {
            source = lib.mkOption {
              type = lib.types.nullOr lib.types.path;
              default = null;
            };
            text = lib.mkOption {
              type = lib.types.nullOr lib.types.lines;
              default = null;
            };
            executable = lib.mkOption {
              type = lib.types.nullOr lib.types.bool;
              default = null;
            };
          };
        }
      );
      default = { };
    };
  };

  nixosStubs = {
    options.environment.systemPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.services.displayManager.sessionPackages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    # Present so the test can prove the module leaves it alone: the
    # never-strand pin is "additive session entry", i.e. this stays null.
    options.services.displayManager.defaultSession = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
    };
  };

  evalHome =
    cfg:
    lib.evalModules {
      modules = [
        ./modules/home.nix
        baseStubs
        homeStubs
        ({ config, ... }: { programs.scoot = cfg; })
      ];
      specialArgs = { inherit pkgs; };
    };

  evalNixos =
    cfg:
    lib.evalModules {
      modules = [
        ./modules/nixos.nix
        baseStubs
        nixosStubs
        ({ config, ... }: { programs.scoot = cfg; })
      ];
      specialArgs = { inherit pkgs; };
    };

  fakePkg = pkgs.runCommand "fake-scoot" { } ''
    mkdir -p $out/bin
    echo '#!/bin/sh' > $out/bin/scoot
    chmod +x $out/bin/scoot
  '';

  # --- home-manager evaluations under test ---
  hmEmpty = evalHome { enable = true; };
  hmFull = evalHome {
    enable = true;
    package = fakePkg;
    settings = {
      layout.gap = 8;
      output.scale = 1.0;
      autostart.commands = [ "spawn waybar" ];
      # Odd keys: `/` forces TOML quoting of the key, and it is NOT a
      # valid keysym (the name is `slash`) -- deliberately so. The
      # round-trip check below proves the quoting is faithful (the key
      # arrives intact), and the live loader proof boots this exact
      # shape through the real compositor, where that one bind is
      # refused per-bind with a warning while the session starts and
      # the other binds load -- the fail-open rule, exercised live.
      binds = {
        "super+t" = "spawn foot";
        "ctrl+alt+space" = "spawn wofi --show drun";
        "super+shift+/" = "close";
      };
    };
    sessionScript = "waybar &\nexec foot\n";
  };
  hmOff = evalHome { enable = false; };
  hmNoPortals = evalHome {
    enable = true;
    portals.enable = false;
  };
  hmRelocated = evalHome {
    enable = true;
    # A different directory than the default `scoot/`, so the script
    # pairing below proves the script follows the config rather than
    # staying at the default path.
    configFile = "myscoot/custom.toml";
    settings.layout.gap = 4;
    sessionScript = "waybar &\nexec foot\n";
  };

  # --- NixOS evaluations under test ---
  osBin = evalNixos {
    enable = true;
    package = fakePkg;
  };
  osSession = evalNixos {
    enable = true;
    package = fakePkg;
    session.enable = true;
  };
  # `session.command` as the common `-- COMMAND` append (e.g. the
  # home-manager module's `sessionScript` output).
  osSessionCmd = evalNixos {
    enable = true;
    package = fakePkg;
    session.enable = true;
    session.command = "${fakePkg}/bin/scoot --tty -- ${fakePkg}/bin/my-shell";
  };
  # `session.command` as the gh-#171 acceptance shape: a wrapper script
  # path replacing the whole Exec line (it launches scoot itself, plus
  # stderr to a log) -- what a plain `--` append cannot express.
  sessionWrapper = pkgs.writeShellScriptBin "scoot-session" ''
    exec ${fakePkg}/bin/scoot --tty -- noctalia-shell 2>/tmp/scoot-session.log
  '';
  osSessionWrapper = evalNixos {
    enable = true;
    package = fakePkg;
    session.enable = true;
    session.command = "${sessionWrapper}/bin/scoot-session";
  };
  # Quoting through the desktop file: the command is rendered verbatim
  # (spaces, quotes, pipes and redirection intact), so what the user
  # wrote is what the greeter launches.
  trickyCmd = "${fakePkg}/bin/scoot --tty -- sh -c 'exec foot 2>/tmp/scoot.log | cat'";
  osSessionQuoting = evalNixos {
    enable = true;
    package = fakePkg;
    session.enable = true;
    session.command = trickyCmd;
  };
  # A command with the entry off installs no entry (the option is inert
  # without `session.enable`).
  osCmdNoSession = evalNixos {
    enable = true;
    package = fakePkg;
    session.command = "${fakePkg}/bin/scoot --tty -- ${fakePkg}/bin/my-shell";
  };
  osOff = evalNixos { enable = false; };

  # --- eval-time structural pins (fail `nix flake check` at eval) ---
  #
  # Note on what these prove: standalone `lib.evalModules` COLLECTS
  # `config.assertions` but does not enforce them (enforcement is the
  # host module system's job -- real `nixos-rebuild` / `home-manager
  # switch` fail loudly on a false one). So instead of relying on
  # enforcement, the pins below assert directly on the collected values:
  # every configuration under test must have all assertions true, and
  # the known-bad combination (session entry with no package) must have
  # a false one. Verified manually that the false assertions carry the
  # helpful messages (see the resolved ticket's evidence record).
  allAssertionsHold = cfg: lib.all (a: a.assertion) cfg.assertions;

  _pins = [
    (
      assert allAssertionsHold hmEmpty.config;
      true
    )
    (
      assert allAssertionsHold hmFull.config;
      true
    )
    (
      assert allAssertionsHold hmOff.config;
      true
    )
    (
      assert allAssertionsHold hmNoPortals.config;
      true
    )
    (
      assert allAssertionsHold hmRelocated.config;
      true
    )
    (
      assert allAssertionsHold osBin.config;
      true
    )
    (
      assert allAssertionsHold osSession.config;
      true
    )
    (
      assert allAssertionsHold osSessionCmd.config;
      true
    )
    (
      assert allAssertionsHold osSessionWrapper.config;
      true
    )
    (
      assert allAssertionsHold osSessionQuoting.config;
      true
    )
    (
      assert allAssertionsHold osCmdNoSession.config;
      true
    )
    (
      assert allAssertionsHold osOff.config;
      true
    )
    # Session entry with no package is refused (both assertions false).
    (
      assert
        !allAssertionsHold
          (evalNixos {
            enable = true;
            session.enable = true;
          }).config;
      true
    )
    # ...likewise with a command set: no package still refuses, so a
    # wrapper-shaped command never renders a broken entry on its own.
    (
      assert
        !allAssertionsHold
          (evalNixos {
            enable = true;
            session.enable = true;
            session.command = "${sessionWrapper}/bin/scoot-session";
          }).config;
      true
    )
    # An explicitly empty command is refused (it would render an empty
    # `Exec=` that fails at the greeter, not at eval).
    (
      assert
        !allAssertionsHold
          (evalNixos {
            enable = true;
            package = fakePkg;
            session.enable = true;
            session.command = "";
          }).config;
      true
    )
    # enable=false manages nothing on either side.
    (
      assert hmOff.config.xdg.configFile == { };
      true
    )
    (
      assert hmOff.config.home.packages == [ ];
      true
    )
    (
      assert osOff.config.environment.systemPackages == [ ];
      true
    )
    (
      assert osOff.config.services.displayManager.sessionPackages == [ ];
      true
    )

    # The session entry is additive: default session untouched...
    (
      assert osSession.config.services.displayManager.defaultSession == null;
      true
    )
    # ...whatever the command (a custom Exec line never pre-selects or
    # auto-runs anything either -- the never-strand rule holds for the
    # wrapper shape too)...
    (
      assert osSessionCmd.config.services.displayManager.defaultSession == null;
      true
    )
    (
      assert osSessionWrapper.config.services.displayManager.defaultSession == null;
      true
    )
    # ...exactly one session package added...
    (
      assert builtins.length osSession.config.services.displayManager.sessionPackages == 1;
      true
    )
    # ...carrying the `providedSessions` nixpkgs' sessionPackages
    # option demands of every element (rejected at eval without it).
    (
      assert
        (builtins.head osSession.config.services.displayManager.sessionPackages).providedSessions
        == [ "scoot" ];
      true
    )
    # ...and nothing added when the session entry stays off.
    (
      assert osBin.config.services.displayManager.sessionPackages == [ ];
      true
    )
    # ...including when a command is set but the entry stays off.
    (
      assert osCmdNoSession.config.services.displayManager.sessionPackages == [ ];
      true
    )

    # Portals file present by default, absent when opted out.
    (
      assert hmEmpty.config.xdg.configFile ? "xdg-desktop-portal/scoot-portals.conf";
      true
    )
    (
      assert !(hmNoPortals.config.xdg.configFile ? "xdg-desktop-portal/scoot-portals.conf");
      true
    )

    # The session script follows the config: a relocated config carries
    # its script beside it, with nothing left at the default path...
    (
      assert hmRelocated.config.xdg.configFile ? "myscoot/session.sh";
      true
    )
    (
      assert !(hmRelocated.config.xdg.configFile ? "scoot/session.sh");
      true
    )
    # ...while the default config keeps the default script path.
    (
      assert hmFull.config.xdg.configFile ? "scoot/session.sh";
      true
    )

    # The representable-but-wrong scoot type (a string for `gap`)
    # type-checks: the refusal happens at session start, fail-safe
    # (whole file discarded for defaults, session still boots) -- NOT
    # as a build-time TOML error. The `wrongTypeToml` content check
    # below proves it renders.
    (
      assert tomlFormat.type.check { layout.gap = "wide"; };
      true
    )
  ];

  # Renders (build succeeds); the live loader proof boots it and shows
  # the session starting on defaults with an error logged.
  wrongTypeToml = tomlFormat.generate "wrong-type.toml" { layout.gap = "wide"; };

  emptyToml = hmEmpty.config.xdg.configFile."scoot/config.toml".source;
  fullToml = hmFull.config.xdg.configFile."scoot/config.toml".source;
  relocatedToml = hmRelocated.config.xdg.configFile."myscoot/custom.toml".source;
  relocatedSessionText = hmRelocated.config.xdg.configFile."myscoot/session.sh".text;
  relocatedSessionExe = hmRelocated.config.xdg.configFile."myscoot/session.sh".executable;
  portalsFile = hmEmpty.config.xdg.configFile."xdg-desktop-portal/scoot-portals.conf".source;
  sessionText = hmFull.config.xdg.configFile."scoot/session.sh".text;
  sessionExe = hmFull.config.xdg.configFile."scoot/session.sh".executable;
  desktopPkg = builtins.head osSession.config.services.displayManager.sessionPackages;
  desktopFile = "${desktopPkg}/share/wayland-sessions/scoot.desktop";
  cmdDesktopFile = "${builtins.head osSessionCmd.config.services.displayManager.sessionPackages}/share/wayland-sessions/scoot.desktop";
  wrapperDesktopFile = "${builtins.head osSessionWrapper.config.services.displayManager.sessionPackages}/share/wayland-sessions/scoot.desktop";
  quotingDesktopFile = "${builtins.head osSessionQuoting.config.services.displayManager.sessionPackages}/share/wayland-sessions/scoot.desktop";
in
assert lib.all (x: x) _pins;
runCommand "scoot-modules-check" { nativeBuildInputs = [ python3 ]; } ''
  set -euo pipefail

  # 1. Empty settings: valid TOML, parses to {} -- a minimal file the
  #    compositor runs as pure defaults.
  python3 -c 'import sys,tomllib; assert tomllib.load(open(sys.argv[1],"rb")) == {}, "empty settings must render an empty table"' ${emptyToml}
  echo "ok: empty settings render a valid minimal file"

  # 2. Full settings incl. odd binds keys: nix -> TOML -> python
  #    round-trips exactly (quoting faithful).
  python3 -c '
  import json,sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  want = json.loads(sys.argv[2])
  assert got == want, f"TOML round-trip mismatch:\n got: {got}\n want: {want}"
  ' ${fullToml} '${builtins.toJSON hmFull.config.programs.scoot.settings}'
  echo "ok: binds with quoting-needing keys round-trip"

  # 3. Relocated config renders under the overridden name with content.
  python3 -c 'import sys,tomllib; assert tomllib.load(open(sys.argv[1],"rb")) == {"layout": {"gap": 4}}' ${relocatedToml}
  echo "ok: configFile override relocates the rendered file"

  # 3b. Relocated session script: written beside the relocated config
  # (the pairing pin -- eval already asserts nothing stays at the
  # default path), shebang + executable bit.
  printf '%s' '${relocatedSessionText}' | head -1 | grep -q '^#!/bin/sh$'
  test "${if relocatedSessionExe then "yes" else "no"}" = yes
  echo "ok: session script follows the relocated config"

  # 4. Portals file is the shipped one, selecting backends.
  grep -q '^default=gtk' ${portalsFile}
  grep -q 'org.freedesktop.impl.portal.ScreenCast=wlr' ${portalsFile}
  echo "ok: portals.conf installed with backend selection"

  # 5. Session script: shebang + executable bit.
  printf '%s' '${sessionText}' | head -1 | grep -q '^#!/bin/sh$'
  test "${if sessionExe then "yes" else "no"}" = yes
  echo "ok: session script written executable with shebang"

  # 6. Session .desktop: launches the configured package on --tty,
  #    names the desktop (which is what sets XDG_CURRENT_DESKTOP when
  #    launched from a display manager).
  grep -q "^Exec=${fakePkg}/bin/scoot --tty$" ${desktopFile}
  grep -q '^DesktopNames=scoot$' ${desktopFile}
  grep -q '^Type=Application$' ${desktopFile}
  echo "ok: wayland-session entry points at the package"

  # 6b. Default is byte-identical with the option present-but-null: the
  # exact-match `$` anchor above already proves no `-- COMMAND` suffix
  # (or anything else) leaked into the bare entry.

  # 6c. session.command as `-- COMMAND` append renders verbatim.
  grep -q "^Exec=${fakePkg}/bin/scoot --tty -- ${fakePkg}/bin/my-shell$" ${cmdDesktopFile}
  echo "ok: session.command appends the session command to Exec"

  # 6d. session.command as a wrapper path (the gh-#171 acceptance
  # shape): the whole Exec line is the wrapper, which launches scoot
  # itself.
  grep -q "^Exec=${sessionWrapper}/bin/scoot-session$" ${wrapperDesktopFile}
  echo "ok: session.command expresses the wrapper-script entry"

  # 6e. Quoting through the desktop file: spaces, quotes, pipes and
  # redirection survive verbatim (fixed-string match, so no regex
  # metacharacter can hide a mangling).
  grep -F "Exec=${trickyCmd}" ${quotingDesktopFile}
  echo "ok: session.command with quoting-needing characters renders verbatim"

  # 7. A representable-but-wrong scoot type renders as TOML (it is the
  #    loader, at session start, that refuses it -- fail-safe).
  grep -q '^gap = "wide"$' ${wrongTypeToml}
  echo "ok: wrong-but-representable types render (refused at startup, not at build)"

  touch $out
  echo "scoot-modules: all file-content checks passed"
''
