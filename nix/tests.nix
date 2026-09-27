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
#   and refuses empty/blank and package-less combinations at eval;
# - the two settings failure modes behave as documented (see below);
# - scootbg for `[wallpaper]` (ticket 10): the NixOS
#   `wallpaper.enable` follows `enable` and installs `wallpaper.package`,
#   off installs nothing, with no package it defaults off and only an
#   explicit `true` fails loudly at eval; the
#   home-manager side installs it whenever `settings.wallpaper` exists and
#   renders `command` as its store path (a user's own `command` wins);
#   the flake wrappers default both packages to the flake's own builds;
#   a macOS home-manager config with a `[wallpaper]` table evaluates, with
#   no scootbg; and the overlay provides `pkgs.scootbg` (Linux only) and
#   is what the pure modules default to.
{
  lib,
  pkgs,
  runCommand,
  python3,
  # The flake's own outputs (`overlays`, `packages`, `homeModule`,
  # `nixosModule`) and aarch64-darwin's package set, from `flake.nix`'s
  # `checks`. Null when this file is evaluated on its own: the checks that
  # need them are then skipped.
  flake ? null,
  darwinPkgs ? null,
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

  # The pure NixOS module with `pkgs` as given. Every configuration
  # below that is not about scootbg gets a stand-in `wallpaper.package`
  # (`evalNixos`), so that `wallpaper.enable` (on by default) cannot make
  # an unrelated pin's assertions fail -- or, worse, make a pin that
  # expects a failure pass for the wrong reason.
  evalNixosWith =
    modules: pkgs': cfg:
    lib.evalModules {
      modules = modules ++ [
        baseStubs
        nixosStubs
        ({ config, ... }: { programs.scoot = cfg; })
      ];
      specialArgs = {
        pkgs = pkgs';
      };
    };
  evalNixosBare = evalNixosWith [ ./modules/nixos.nix ] pkgs;
  evalNixos = evalNixosWith [
    ./modules/nixos.nix
    { programs.scoot.wallpaper.package = lib.mkDefault fakeBg; }
  ] pkgs;

  fakePkg = pkgs.runCommand "fake-scoot" { } ''
    mkdir -p $out/bin
    echo '#!/bin/sh' > $out/bin/scoot
    chmod +x $out/bin/scoot
  '';
  fakeBg = pkgs.runCommand "fake-scootbg" { } ''
    mkdir -p $out/bin
    echo '#!/bin/sh' > $out/bin/scootbg
    chmod +x $out/bin/scootbg
  '';

  drvs = map (p: p.drvPath);
  system = pkgs.stdenv.hostPlatform.system;
  isLinux = pkgs.stdenv.hostPlatform.isLinux;
  failing = cfg: map (a: a.message) (lib.filter (a: !a.assertion) cfg.assertions);

  # --- scootbg ([wallpaper]) evaluations under test ---
  # NixOS: on by default with `enable`, installing the package.
  osWallOn = evalNixos {
    enable = true;
    package = fakePkg;
  };
  # NixOS: opted out.
  osWallOff = evalNixos {
    enable = true;
    package = fakePkg;
    wallpaper.enable = false;
  };
  # NixOS: scootbg alone, for another compositor.
  osWallOnly = evalNixos { wallpaper.enable = true; };
  # NixOS, direct module, no overlay, no package: off by default, so an
  # upgrade breaks nobody's evaluation...
  osWallNoPkg = evalNixosBare {
    enable = true;
    package = fakePkg;
  };
  # ...while asking for it explicitly with no package is loud.
  osWallNoPkgExplicit = evalNixosBare {
    enable = true;
    package = fakePkg;
    wallpaper.enable = true;
  };
  # Home-manager: a `[wallpaper]` table installs scootbg and gets its path.
  hmWall = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    settings.wallpaper = {
      image = "~/Pictures/hills.jpg";
      output."DP-2".color = "#101014";
    };
  };
  # ...a `command` the user set wins.
  hmWallOwnCommand = evalHome {
    enable = true;
    wallpaper.package = fakeBg;
    settings.wallpaper = {
      color = "#1e1e2e";
      command = "/opt/scootbg/bin/scootbg";
    };
  };
  # ...no table: nothing installed, nothing added.
  hmNoWall = evalHome {
    enable = true;
    wallpaper.package = fakeBg;
    settings.layout.gap = 4;
  };
  # ...opted out: the table renders as written, nothing installed.
  hmWallOff = evalHome {
    enable = true;
    wallpaper.enable = false;
    wallpaper.package = fakeBg;
    settings.wallpaper.color = "#1e1e2e";
  };
  # ...a `wallpaper` that is not a table renders as written (scoot refuses
  # it by name at startup) rather than failing evaluation.
  hmWallNotTable = evalHome {
    enable = true;
    wallpaper.package = fakeBg;
    settings.wallpaper = "blue";
  };

  # --- the flake's wrappers, overlay and Darwin (only from flake.nix) ---
  withFlake = flake != null;
  built = flake.packages.${system} or { };
  evalHomeWith =
    module: pkgs': cfg:
    lib.evalModules {
      modules = [
        module
        baseStubs
        homeStubs
        ({ config, ... }: { programs.scoot = cfg; })
      ];
      specialArgs = {
        pkgs = pkgs';
      };
    };
  # A flake consumer: `enable` and a `[wallpaper]` table, nothing else.
  osFlake = evalNixosWith [ flake.nixosModule ] pkgs { enable = true; };
  hmFlake = evalHomeWith flake.homeModule pkgs {
    enable = true;
    settings.wallpaper.color = "#1e1e2e";
  };
  # The same home configuration on a Mac: evaluates, installs nothing,
  # renders the table as written for the Linux box it deploys to.
  hmDarwin = evalHomeWith flake.homeModule darwinPkgs {
    enable = true;
    settings.wallpaper.image = "~/Pictures/hills.jpg";
  };
  # The overlay, and the pure module defaulting to what it provides.
  overlaid = pkgs.appendOverlays [ flake.overlays.default ];
  osOverlaid = evalNixosWith [ ./modules/nixos.nix ] overlaid { enable = true; };
  darwinOverlaid = darwinPkgs.appendOverlays [ flake.overlays.default ];
  sorted = packages: lib.sort lib.lessThan (drvs packages);

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
    # ...and so is a whitespace-only one (it would render an `Exec=`
    # of nothing but blanks that fails the same way).
    (
      assert
        !allAssertionsHold
          (evalNixos {
            enable = true;
            package = fakePkg;
            session.enable = true;
            session.command = "   ";
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

    # --- scootbg ([wallpaper]) ---
    # NixOS: on with `enable`, the package installed beside scoot.
    (
      assert allAssertionsHold osWallOn.config;
      true
    )
    (
      assert osWallOn.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      assert
        sorted osWallOn.config.environment.systemPackages == sorted [
          fakePkg
          fakeBg
        ];
      true
    )
    # ...off: nothing of scootbg, and no package needed.
    (
      assert allAssertionsHold osWallOff.config;
      true
    )
    (
      assert drvs osWallOff.config.environment.systemPackages == drvs [ fakePkg ];
      true
    )
    # ...follows `enable`: off with scoot off.
    (
      assert !osOff.config.programs.scoot.wallpaper.enable;
      true
    )
    # ...on its own, without scoot.
    (
      assert allAssertionsHold osWallOnly.config;
      true
    )
    (
      assert drvs osWallOnly.config.environment.systemPackages == drvs [ fakeBg ];
      true
    )
    # ...direct module, no overlay, no package: off by default, every
    # assertion holds, only scoot installed (N4, review of PR #297)...
    (
      assert !osWallNoPkg.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      assert allAssertionsHold osWallNoPkg.config;
      true
    )
    (
      assert drvs osWallNoPkg.config.environment.systemPackages == drvs [ fakePkg ];
      true
    )
    # ...and enabled explicitly with no package: exactly one failing
    # assertion, naming the option to set.
    (
      assert builtins.length (failing osWallNoPkgExplicit.config) == 1;
      true
    )
    (
      assert lib.hasInfix "programs.scoot.wallpaper.package is null" (
        builtins.head (failing osWallNoPkgExplicit.config)
      );
      true
    )
    # Home-manager: a `[wallpaper]` table installs scootbg...
    (
      assert allAssertionsHold hmWall.config;
      true
    )
    (
      assert hmWall.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      assert
        drvs hmWall.config.home.packages == drvs [
          fakePkg
          fakeBg
        ];
      true
    )
    (
      assert drvs hmWallOwnCommand.config.home.packages == drvs [ fakeBg ];
      true
    )
    # ...no table: nothing.
    (
      assert !hmNoWall.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      assert hmNoWall.config.home.packages == [ ];
      true
    )
    # ...opted out: nothing installed.
    (
      assert hmWallOff.config.home.packages == [ ];
      true
    )
    # ...not a table: evaluates (and installs; nothing is injected).
    (
      assert drvs hmWallNotTable.config.home.packages == drvs [ fakeBg ];
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

  # The flake's own outputs, when evaluated from flake.nix.
  _flakePins =
    lib.optionals withFlake [
      (
        assert allAssertionsHold hmFlake.config;
        true
      )
    ]
    # NixOS is Linux: on this check's Darwin run there is no scootbg to
    # install, and no NixOS to install it on.
    ++ lib.optionals (withFlake && isLinux) [
      # A flake NixOS consumer who only sets `enable`: scoot and scootbg,
      # the flake's own builds, and every assertion holds.
      (
        assert allAssertionsHold osFlake.config;
        true
      )
      (
        assert
          sorted osFlake.config.environment.systemPackages == sorted [
            built.default
            built.scootbg
          ];
        true
      )
      (
        assert
          drvs hmFlake.config.home.packages == drvs [
            built.default
            built.scootbg
          ];
        true
      )
      # The overlay is the flake's own builds...
      (
        assert overlaid.scootbg.drvPath == built.scootbg.drvPath;
        true
      )
      (
        assert overlaid.scoot.drvPath == built.scoot.drvPath;
        true
      )
      (
        assert overlaid.scootctl.drvPath == built.scootctl.drvPath;
        true
      )
      # ...and what the pure module defaults to, with no package set.
      (
        assert allAssertionsHold osOverlaid.config;
        true
      )
      (
        assert
          sorted osOverlaid.config.environment.systemPackages == sorted [
            overlaid.scoot
            overlaid.scootbg
          ];
        true
      )
    ]
    ++ lib.optionals (withFlake && !isLinux) [
      # This check run on Darwin itself: no scootbg, nothing installed.
      (
        assert hmFlake.config.programs.scoot.wallpaper.package == null;
        true
      )
      (
        assert hmFlake.config.home.packages == [ ];
        true
      )
    ]
    ++ lib.optionals (withFlake && darwinPkgs != null) [
      # A macOS home configuration with a `[wallpaper]` table evaluates,
      # with no scootbg (Linux-only) and no binary: files only.
      (
        assert allAssertionsHold hmDarwin.config;
        true
      )
      (
        assert hmDarwin.config.programs.scoot.wallpaper.package == null;
        true
      )
      (
        assert hmDarwin.config.home.packages == [ ];
        true
      )
      (
        assert builtins.isString hmDarwin.config.xdg.configFile."scoot/config.toml".source.drvPath;
        true
      )
      # The overlay gives Darwin the client, and no scootbg.
      (
        assert !(darwinOverlaid ? scootbg);
        true
      )
      (
        assert darwinOverlaid.scootctl.drvPath == flake.packages.aarch64-darwin.scootctl.drvPath;
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
  hmWallToml = hmWall.config.xdg.configFile."scoot/config.toml".source;
  hmWallOwnCommandToml = hmWallOwnCommand.config.xdg.configFile."scoot/config.toml".source;
  hmWallOffToml = hmWallOff.config.xdg.configFile."scoot/config.toml".source;
  hmNoWallToml = hmNoWall.config.xdg.configFile."scoot/config.toml".source;
  hmWallNotTableToml = hmWallNotTable.config.xdg.configFile."scoot/config.toml".source;
in
assert lib.all (x: x) _pins;
assert lib.all (x: x) _flakePins;
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

  # 8. [wallpaper]: `command` is the installed scootbg's store path, the
  #    rest of the table as written...
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))["wallpaper"]
  want = {"command": sys.argv[2], "image": "~/Pictures/hills.jpg", "output": {"DP-2": {"color": "#101014"}}}
  assert got == want, f"{got} != {want}"
  ' ${hmWallToml} '${fakeBg}/bin/scootbg'
  echo "ok: [wallpaper] command points at the installed scootbg"
  #    ...a command the user set wins...
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))["wallpaper"]
  assert got == {"color": "#1e1e2e", "command": "/opt/scootbg/bin/scootbg"}, got
  ' ${hmWallOwnCommandToml}
  echo "ok: a [wallpaper] command the user set is kept"
  #    ...opted out, the table renders as written...
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  assert got == {"wallpaper": {"color": "#1e1e2e"}}, got
  ' ${hmWallOffToml}
  echo "ok: wallpaper.enable = false renders the table as written"
  #    ...no table, none is added; not a table, it renders as written.
  python3 -c '
  import sys,tomllib
  assert tomllib.load(open(sys.argv[1],"rb")) == {"layout": {"gap": 4}}
  assert tomllib.load(open(sys.argv[2],"rb")) == {"wallpaper": "blue"}
  ' ${hmNoWallToml} ${hmWallNotTableToml}
  echo "ok: no [wallpaper] table is added, and a non-table renders as written"

  touch $out
  echo "scoot-modules: all file-content checks passed"
''
