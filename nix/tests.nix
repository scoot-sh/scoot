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
#   with no command it runs the `scoot-session` launcher (the session
#   wiring: user-manager import, `graphical-session.target`, the
#   activation environment, teardown), whose user units
#   (`scoot.service` with `BindsTo`/`Before` the session target and an
#   `ExecStart` naming the package's binary, plus
#   `scoot-shutdown.target` conflicting the session targets) are
#   installed beside the entry and absent with it off;
# - `session.command` renders verbatim into `Exec=` (launcher default,
#   `-- COMMAND` append, wrapper-script path, quoting
#   with spaces/quotes/pipes intact), stays inert with the entry off,
#   and refuses empty/blank and package-less combinations at eval;
# - the greeter (Linux only: it imports nixpkgs' own regreet module):
#   `greeter.enable` turns on `services.displayManager.regreet`, forces
#   the session entry (and its units) on beside it, leaves the backdrop
#   alone by default and renders a set one as ReGreet's
#   `background.path`; off changes nothing; GDM/SDDM, an explicitly
#   disabled session entry, a missing `enable`, and a Stylix-owned
#   backdrop are each refused at eval, naming the conflict;
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
# - scootbar (docs/scootbar/backlog/resolved/nix-package-done.md): the
#   overlay provides `pkgs.scootbar` on Linux, the flake's own build, and
#   no `scootbar-demo`; on Darwin neither the overlay nor `packages` has
#   one; its Cargo features reach the build through `.override`; the
#   demo is a separate derivation running the bare bar; and no input of
#   `scootbar` names a font (its built closure is checked by
#   .github/workflows/nix-build.yml).
# - Stylix for the compositor (`programs.scoot.stylix.enable`, on by
#   default): the `[appearance]` ring/background colors from base16
#   (`base0D`/`base03`/`base00`, what Stylix's own sway, hyprland and
#   river targets use), `cursor_theme`/`cursor_size` from
#   `stylix.cursor`, `[wallpaper]` `image`/`mode` from `stylix.image` /
#   `stylix.imageScalingMode` (same five modes as scootbg's); each at
#   its own leaf so a user value wins per key; either switch off
#   (`stylix.enable`, `programs.scoot.stylix.enable`) is as if Stylix
#   were absent; with no `cursor` no cursor keys appear, with no `image`
#   no `[wallpaper]` table is added (so no scootbg is installed for it);
#   the cursor package is never installed (Stylix's own cursor target
#   owns that); and the module evaluates with no Stylix option defined
#   at all (every pre-existing evaluation below does exactly that).
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
    # `config.lib` is an option in both real module systems. Declared so
    # the Stylix detection (`config.lib ? stylix`, as in
    # `nix/modules/scootbar.nix`) has something to ask: without Stylix
    # it is empty and every Stylix default stays off.
    options.lib = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
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
    # The login screens the greeter option is checked against: GDM and
    # SDDM must be off for greetd/ReGreet (refused at eval). Plain bools
    # defaulting to off, like the real modules.
    options.services.displayManager.gdm.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
    };
    options.services.displayManager.sddm.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
    };
    # The launcher's user units (`scoot.service`, `scoot-shutdown.target`).
    # An attrs-of-lineshape, like the real `systemd.user.units`: the pins
    # below read `.text` out of each.
    options.systemd.user.units = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options = {
            text = lib.mkOption {
              type = lib.types.nullOr lib.types.lines;
              default = null;
            };
          };
        }
      );
      default = { };
    };
  };

  # Stand-ins for what nixpkgs' own regreet module
  # (`services/display-managers/regreet.nix` at the pinned rev, imported
  # by `nix/modules/nixos.nix` itself) sets and reads. These ride along
  # in every NixOS evaluation through `evalNixosWith`, greeter or not.
  # The pins below read `enable` and `settings.background.path` out of
  # the real module -- so the wiring is checked against nixpkgs' option,
  # not a copy of it (which is also what catches a future rename like
  # the `programs.regreet` one this rev already carries as an alias) --
  # and the module's own assertion (the login user exists) must
  # evaluate, which is what the `user` default is for (matching nixpkgs
  # greetd's own `mkDefault "greeter"`). Nothing here is built:
  # evaluation only.
  regreetStubs = {
    options.services.greetd.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
    };
    options.services.greetd.settings = lib.mkOption {
      type = lib.types.submodule {
        options.default_session = lib.mkOption {
          type = lib.types.submodule {
            options.command = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
            };
            options.user = lib.mkOption {
              type = lib.types.str;
              default = "greeter";
            };
          };
          default = { };
        };
      };
      default = { };
    };
    options.services.accounts-daemon.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
    };
    options.environment.etc = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.submodule {
          options.source = lib.mkOption {
            type = lib.types.nullOr lib.types.path;
            default = null;
          };
          options.text = lib.mkOption {
            type = lib.types.nullOr lib.types.lines;
            default = null;
          };
        }
      );
      default = { };
    };
    options.systemd.tmpfiles.settings = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    options.fonts.packages = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.users.users = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
  };

  # The one Stylix leaf the greeter's `background` assertion reads
  # (image set, regreet target on). Only the conflict-case evaluation
  # declares it; every other greeter evaluation runs with no Stylix at
  # all, which is what proves the `config.stylix or {}` fallback.
  stylixRegreetStub = {
    options.stylix.image = lib.mkOption {
      type = lib.types.nullOr lib.types.raw;
      default = null;
    };
    options.stylix.targets.regreet.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
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

  # The things the scoot module reads of Stylix, as Stylix defines them
  # (checked by hand against nix-community/stylix at fb28acd, the rev
  # `nix/scootbar-tests.nix` names: `stylix/palette.nix` for `image` and
  # `imageScalingMode`, `stylix/cursor.nix` for `cursor`, the palette
  # module for `lib.stylix.colors`). A rename there would pass this file
  # and miss on a real Stylix; the color, cursor and image reads are the
  # sites to look at. `image` is a store path, as Stylix's `pathInStore`
  # coercion makes it.
  styImage = builtins.toFile "stylix-wallpaper.png" "fake wallpaper";
  fakeCursorPkg = pkgs.runCommand "fake-cursor-theme" { } ''
    mkdir -p $out/share/icons/Vanilla-DMZ/cursors
    : > $out/share/icons/Vanilla-DMZ/cursors/left_ptr
  '';
  stylixStub =
    {
      enable ? true,
      cursor ? {
        name = "Vanilla-DMZ";
        package = fakeCursorPkg;
        size = 24;
      },
      image ? styImage,
      mode ? "fill",
    }:
    {
      options.stylix.enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
      };
      options.stylix.cursor = lib.mkOption {
        type = lib.types.nullOr lib.types.raw;
        default = null;
      };
      options.stylix.image = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
      };
      options.stylix.imageScalingMode = lib.mkOption {
        type = lib.types.str;
        default = "fill";
      };
      config = {
        stylix.enable = enable;
        stylix.cursor = cursor;
        stylix.image = image;
        stylix.imageScalingMode = mode;
        lib.stylix.colors.withHashtag = {
          base00 = "#101010";
          base03 = "#303030";
          base0D = "#0000ff";
        };
      };
    };

  evalHomeStylix =
    stub: cfg:
    lib.evalModules {
      modules = [
        ./modules/home.nix
        baseStubs
        homeStubs
        (stylixStub stub)
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
        # Beside our own module, which imports nixpkgs' regreet module
        # (see `imports` in `nix/modules/nixos.nix`): its surroundings
        # ride along in every NixOS evaluation, greeter or not, the
        # same way `nixosStubs` rides along for NixOS-owned options.
        regreetStubs
        ({ config, ... }: { programs.scoot = cfg; })
      ];
      specialArgs = {
        pkgs = pkgs';
        modulesPath = "${pkgs'.path}/nixos/modules";
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

  # --- Stylix evaluations under test ---
  # Themed: colors, cursor and wallpaper from the stub. `package` and
  # `wallpaper.package` set so the install pins below mean something.
  hmStylix = evalHomeStylix { } {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
  };
  # A non-default scaling mode passes through verbatim.
  hmStylixTile = evalHomeStylix { mode = "tile"; } {
    enable = true;
  };
  # Stylix's own switch off: as if Stylix were absent.
  hmStylixDisabled = evalHomeStylix { enable = false; } {
    enable = true;
  };
  # The module's own switch off: as if Stylix were absent.
  hmStylixOptOut = evalHomeStylix { } {
    enable = true;
    stylix.enable = false;
  };
  # No cursor set (Stylix without one): no cursor keys, colors stay.
  hmStylixNoCursor = evalHomeStylix { cursor = null; } {
    enable = true;
  };
  # No image set: no `[wallpaper]` table added (and so no scootbg), colors
  # and cursor stay.
  hmStylixNoImage = evalHomeStylix { image = null; } {
    enable = true;
    wallpaper.package = fakeBg;
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
  # The flake's NixOS module inside a real NixOS evaluation (nixpkgs' own
  # `eval-config.nix`: the full module list, `pkgs` from `_module.args`
  # as every NixOS system gets it). The stub evaluations above hand
  # `pkgs` in through `specialArgs`, which hides a module reading `pkgs`
  # where `_module.args` is not yet available -- `imports` above all,
  # where it is infinite recursion on every real system (the regression
  # #416 shipped: importing the module failed, greeter or not). Only
  # options are read, so nothing is built and no root file system is
  # needed.
  evalRealNixos =
    cfg:
    import "${pkgs.path}/nixos/lib/eval-config.nix" {
      system = null;
      modules = [
        flake.nixosModule
        {
          nixpkgs.hostPlatform = system;
          programs.scoot = cfg;
        }
      ];
    };
  osRealImported = evalRealNixos { };
  osRealGreeter = evalRealNixos {
    enable = true;
    greeter.enable = true;
  };
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
  # `lib.hasInfix` over strings that name store paths: the check is of
  # text, so their context (the derivations they refer to) is dropped, which
  # the regex builtin under `hasInfix` otherwise refuses.
  contains =
    needle: haystack:
    lib.hasInfix (builtins.unsafeDiscardStringContext needle) (
      builtins.unsafeDiscardStringContext haystack
    );
  # scootbar's Cargo features through `.override` (nix/scootbar.nix).
  scootbarNoModules = built.scootbar.override { buildNoDefaultFeatures = true; };
  scootbarClockOnly = built.scootbar.override {
    buildNoDefaultFeatures = true;
    buildFeatures = [ "clock" ];
  };
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

  # --- greeter evaluations under test (Linux only: a system-level
  # display-manager wiring with no meaning on Darwin) ---
  #
  # No explicit regreet import here: `nix/modules/nixos.nix` imports
  # nixpkgs' own `services/display-managers/regreet.nix` itself (see its
  # `imports`), so the pins below check agreement with nixpkgs' real
  # option -- not a copy of it, which is also what catches a future
  # rename the way this rev's `programs.regreet` alias would need.
  evalNixosRegreet =
    cfg: extra:
    lib.evalModules {
      modules = [
        ./modules/nixos.nix
        baseStubs
        nixosStubs
        regreetStubs
        { programs.scoot.wallpaper.package = lib.mkDefault fakeBg; }
        ({ config, ... }: { programs.scoot = cfg; } // extra)
      ];
      specialArgs = {
        pkgs = pkgs;
        modulesPath = "${pkgs.path}/nixos/modules";
      };
    };
  # What a real configuration provides too: nixpkgs' regreet module
  # refuses a missing login user at eval, so the test user exists.
  greeterUser = {
    users.users.greeter = {
      isNormalUser = true;
    };
  };
  # The greeter on: ReGreet enabled, the session entry forced on, the
  # launcher's units beside it, no backdrop.
  osGreeter = evalNixosRegreet {
    enable = true;
    package = fakePkg;
    greeter.enable = true;
  } greeterUser;
  # ...with a backdrop: ReGreet's `background.path` is the image.
  wallpaperImage = builtins.toFile "greeter-bg.png" "fake background";
  osGreeterBg = evalNixosRegreet {
    enable = true;
    package = fakePkg;
    greeter.enable = true;
    greeter.background = wallpaperImage;
  } greeterUser;
  # ...off: no ReGreet, and (no session entry either) nothing installed.
  osGreeterOff = evalNixosRegreet {
    enable = true;
    package = fakePkg;
  } greeterUser;
  # ...against GDM: refused, naming the conflict.
  osGreeterGdm = evalNixosRegreet {
    enable = true;
    package = fakePkg;
    greeter.enable = true;
  } (greeterUser // { services.displayManager.gdm.enable = true; });
  # ...against SDDM: refused the same way.
  osGreeterSddm = evalNixosRegreet {
    enable = true;
    package = fakePkg;
    greeter.enable = true;
  } (greeterUser // { services.displayManager.sddm.enable = true; });
  # ...with the session entry explicitly off: refused (a greeter with no
  # scoot to offer).
  osGreeterNoSession = evalNixosRegreet {
    enable = true;
    package = fakePkg;
    greeter.enable = true;
    session.enable = false;
  } greeterUser;
  # ...without scoot itself: refused.
  osGreeterNoEnable = evalNixosRegreet {
    package = fakePkg;
    greeter.enable = true;
  } greeterUser;
  # ...with a backdrop beside Stylix's regreet image theming: refused
  # (two owners for one backdrop). The stub declares only the one Stylix
  # leaf the assertion reads; its absence everywhere else is the
  # no-Stylix case the other evaluations prove.
  osGreeterStylix = lib.evalModules {
    modules = [
      ./modules/nixos.nix
      baseStubs
      nixosStubs
      regreetStubs
      stylixRegreetStub
      { programs.scoot.wallpaper.package = lib.mkDefault fakeBg; }
      (
        { config, ... }:
        {
          programs.scoot = {
            enable = true;
            package = fakePkg;
            greeter.enable = true;
            greeter.background = wallpaperImage;
          };
          stylix.image = styImage;
          stylix.targets.regreet.enable = true;
        }
        // greeterUser
      )
    ];
    specialArgs = {
      pkgs = pkgs;
      modulesPath = "${pkgs.path}/nixos/modules";
    };
  };

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

    # The launcher's units ride with the entry: both installed...
    (
      assert osSession.config.systemd.user.units ? "scoot.service";
      true
    )
    (
      assert osSession.config.systemd.user.units ? "scoot-shutdown.target";
      true
    )
    # ...the service bound to the session target it pulls in, and
    # ordered before it (so the target activates once the service
    # starts, and stopping the target stops the service)...
    (
      assert lib.hasInfix "BindsTo=graphical-session.target" sessionServiceText;
      true
    )
    (
      assert lib.hasInfix "Before=graphical-session.target" sessionServiceText;
      true
    )
    # ...launching this package's binary on `--tty` (the same build the
    # entry names, wrapper included)...
    (
      assert contains "ExecStart=${fakePkg}/bin/scoot --tty" sessionServiceText;
      true
    )
    # ...and the shutdown target conflicting the session targets away
    # (which is what stops session-bound units when scoot exits).
    (
      assert lib.hasInfix "Conflicts=graphical-session.target" sessionShutdownText;
      true
    )
    # ...while no entry means no units either (a command with the entry
    # off, or scoot off entirely).
    (
      assert osBin.config.systemd.user.units == { };
      true
    )
    (
      assert osCmdNoSession.config.systemd.user.units == { };
      true
    )
    (
      assert osOff.config.systemd.user.units == { };
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

    # --- Stylix (`programs.scoot.stylix.enable`) ---
    # On: the ring and background colors from base16, the cursor name
    # and size from `stylix.cursor`, the wallpaper image and mode from
    # `stylix.image` / `stylix.imageScalingMode`.
    (
      assert allAssertionsHold hmStylix.config;
      true
    )
    (
      assert
        hmStylix.config.programs.scoot.settings == {
          appearance = {
            focus_ring_active_color = "#0000ff";
            focus_ring_inactive_color = "#303030";
            background_color = "#101010";
            cursor_theme = "Vanilla-DMZ";
            cursor_size = 24;
          };
          wallpaper = {
            image = "${styImage}";
            mode = "fill";
          };
        };
      true
    )
    # A non-default scaling mode passes through verbatim.
    (
      assert hmStylixTile.config.programs.scoot.settings.wallpaper.mode == "tile";
      true
    )
    # Either switch off is as if Stylix were absent: no key appears.
    (
      assert hmStylixDisabled.config.programs.scoot.settings == { };
      true
    )
    (
      assert hmStylixOptOut.config.programs.scoot.settings == { };
      true
    )
    # Every Stylix leaf, one user override at a time: each yields to the
    # user while the rest stay Stylix's (a leaf defined without mkDefault
    # would conflict, and fail here).
    (
      assert lib.all
        (
          leaf:
          let
            s =
              (evalHomeStylix { } {
                enable = true;
                settings.${leaf.table}.${leaf.token} = leaf.value;
              }).config.programs.scoot.settings;
            t = hmStylix.config.programs.scoot.settings;
          in
          s.${leaf.table}.${leaf.token} == leaf.value
          &&
            builtins.removeAttrs s.${leaf.table} [ leaf.token ]
            == builtins.removeAttrs t.${leaf.table} [ leaf.token ]
        )
        [
          {
            table = "appearance";
            token = "focus_ring_active_color";
            value = "#abcdef";
          }
          {
            table = "appearance";
            token = "focus_ring_inactive_color";
            value = "#bcdefa";
          }
          {
            table = "appearance";
            token = "background_color";
            value = "#cdefab";
          }
          {
            table = "appearance";
            token = "cursor_theme";
            value = "Adwaita";
          }
          {
            table = "appearance";
            token = "cursor_size";
            value = 32;
          }
          {
            table = "wallpaper";
            token = "image";
            value = "/user/wall.png";
          }
          {
            table = "wallpaper";
            token = "mode";
            value = "center";
          }
        ];
      true
    )
    # No cursor set: no cursor keys, the colors stay.
    (
      assert
        hmStylixNoCursor.config.programs.scoot.settings.appearance == {
          focus_ring_active_color = "#0000ff";
          focus_ring_inactive_color = "#303030";
          background_color = "#101010";
        };
      true
    )
    # No image set: no `[wallpaper]` table is added, so `wallpaper.enable`
    # stays off and no scootbg is installed for it.
    (
      assert !(hmStylixNoImage.config.programs.scoot.settings ? wallpaper);
      true
    )
    (
      assert !hmStylixNoImage.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      assert hmStylixNoImage.config.home.packages == [ ];
      true
    )
    # A Stylix image turns `wallpaper.enable` on (it follows
    # `settings ? wallpaper`) and installs scootbg beside scoot -- but
    # never the cursor package, which Stylix's own cursor target owns.
    (
      assert hmStylix.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      assert
        sorted hmStylix.config.home.packages == sorted [
          fakePkg
          fakeBg
        ];
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
      # Merely importing the module into a real NixOS evaluates (the
      # M2's shape: imported, nothing enabled), and the greeter wires
      # nixpkgs' own greetd and ReGreet there, not only in the stubs.
      (
        assert !osRealImported.config.programs.scoot.enable;
        assert !osRealImported.config.services.greetd.enable;
        true
      )
      (
        assert osRealGreeter.config.services.displayManager.regreet.enable;
        assert osRealGreeter.config.services.greetd.enable;
        assert osRealGreeter.config.systemd.user.units ? "scoot.service";
        true
      )
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
      (
        assert overlaid.scootbar.drvPath == built.scootbar.drvPath;
        true
      )
      # The demo is something to run, not to build on: not in the overlay.
      (
        assert !(overlaid ? scootbar-demo);
        true
      )
      # scootbar's Cargo features are `.override` arguments that reach the
      # cargo hook: the default build names none (the crate's default,
      # the clock), and a build without modules turns the defaults off.
      (
        assert !built.scootbar.cargoBuildNoDefaultFeatures && built.scootbar.cargoBuildFeatures == [ ];
        true
      )
      (
        assert scootbarNoModules.cargoBuildNoDefaultFeatures && scootbarNoModules.cargoBuildFeatures == [ ];
        true
      )
      (
        assert
          scootbarClockOnly.cargoBuildNoDefaultFeatures
          && scootbarClockOnly.cargoBuildFeatures == [ "clock" ];
        true
      )
      (
        assert scootbarNoModules.drvPath != built.scootbar.drvPath;
        true
      )
      # One binary named for the bar in each, and the demo runs the bare
      # build (it is a wrapper, not a second compile) with the font as a
      # separate store path; no input of `scootbar` names a font.
      (
        assert lib.getExe built.scootbar == "${built.scootbar}/bin/scootbar";
        true
      )
      (
        assert lib.getExe built.scootbar-demo == "${built.scootbar-demo}/bin/scootbar";
        true
      )
      (
        assert contains (lib.getExe built.scootbar) built.scootbar-demo.text;
        true
      )
      (
        assert contains "${pkgs.dejavu_fonts.minimal}/share/fonts/truetype/DejaVuSans.ttf"
          built.scootbar-demo.text;
        true
      )
      (
        assert lib.all (input: !(contains "font" "${input}")) (
          built.scootbar.buildInputs
          ++ built.scootbar.nativeBuildInputs
          ++ built.scootbar.propagatedBuildInputs
        );
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
      # The overlay gives Darwin the client, and no scootbg or scootbar.
      (
        assert !(darwinOverlaid ? scootbg);
        true
      )
      (
        assert !(darwinOverlaid ? scootbar);
        true
      )
      (
        assert !(flake.packages.aarch64-darwin ? scootbar || flake.packages.aarch64-darwin ? scootbar-demo);
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
  # The launcher's units, as the entry's configuration renders them.
  sessionServiceText = osSession.config.systemd.user.units."scoot.service".text;
  sessionShutdownText = osSession.config.systemd.user.units."scoot-shutdown.target".text;
  cmdDesktopFile = "${builtins.head osSessionCmd.config.services.displayManager.sessionPackages}/share/wayland-sessions/scoot.desktop";
  wrapperDesktopFile = "${builtins.head osSessionWrapper.config.services.displayManager.sessionPackages}/share/wayland-sessions/scoot.desktop";
  quotingDesktopFile = "${builtins.head osSessionQuoting.config.services.displayManager.sessionPackages}/share/wayland-sessions/scoot.desktop";
  hmWallToml = hmWall.config.xdg.configFile."scoot/config.toml".source;
  hmWallOwnCommandToml = hmWallOwnCommand.config.xdg.configFile."scoot/config.toml".source;
  hmWallOffToml = hmWallOff.config.xdg.configFile."scoot/config.toml".source;
  hmNoWallToml = hmNoWall.config.xdg.configFile."scoot/config.toml".source;
  hmWallNotTableToml = hmWallNotTable.config.xdg.configFile."scoot/config.toml".source;
  hmStylixToml = hmStylix.config.xdg.configFile."scoot/config.toml".source;

  # --- greeter structural pins (fail `nix flake check` at eval) ---
  #
  # Linux only: these import nixpkgs' own regreet module, a system-level
  # display-manager wiring with no meaning on Darwin (same gating as the
  # flake/overlay pins below). Each refusal pin also proves the message
  # names the conflict (the `hasInfix` half), not just that something
  # fails.
  _greeterPins = lib.optionals isLinux [
    # On: every assertion holds (including nixpkgs regreet's own login
    # user one)...
    (
      assert allAssertionsHold osGreeter.config;
      true
    )
    # ...ReGreet enabled through nixpkgs' option (not an alias)...
    (
      assert osGreeter.config.services.displayManager.regreet.enable;
      true
    )
    # ...the session entry forced on beside it (what the greeter lists)...
    (
      assert builtins.length osGreeter.config.services.displayManager.sessionPackages == 1;
      true
    )
    # ...the launcher's units beside that...
    (
      assert osGreeter.config.systemd.user.units ? "scoot.service";
      true
    )
    (
      assert osGreeter.config.systemd.user.units ? "scoot-shutdown.target";
      true
    )
    # ...and no backdrop by default (ReGreet's own stands, and Stylix
    # with its regreet target sets it from `stylix.image`).
    (
      assert !(osGreeter.config.services.displayManager.regreet.settings ? background);
      true
    )
    # With a backdrop: ReGreet's `background.path` is the image, and
    # every assertion still holds.
    (
      assert allAssertionsHold osGreeterBg.config;
      true
    )
    (
      assert contains "${
        wallpaperImage
      }" osGreeterBg.config.services.displayManager.regreet.settings.background.path;
      true
    )
    # Off changes nothing: no ReGreet, and (no session entry either) no
    # entry and no units.
    (
      assert !osGreeterOff.config.services.displayManager.regreet.enable;
      true
    )
    (
      assert osGreeterOff.config.services.displayManager.sessionPackages == [ ];
      true
    )
    (
      assert osGreeterOff.config.systemd.user.units == { };
      true
    )
    (
      assert allAssertionsHold osGreeterOff.config;
      true
    )
    # Against GDM: exactly one failing assertion, naming the conflict.
    (
      assert builtins.length (failing osGreeterGdm.config) == 1;
      true
    )
    (
      assert lib.hasInfix "conflicts with GDM" (builtins.head (failing osGreeterGdm.config));
      true
    )
    # Against SDDM: the same.
    (
      assert builtins.length (failing osGreeterSddm.config) == 1;
      true
    )
    (
      assert lib.hasInfix "conflicts with SDDM" (builtins.head (failing osGreeterSddm.config));
      true
    )
    # With the session entry explicitly off: refused (a greeter with no
    # scoot to offer), and the message says so.
    (
      assert builtins.length (failing osGreeterNoSession.config) == 1;
      true
    )
    (
      assert lib.hasInfix "needs\nprograms.scoot.session.enable" (
        builtins.head (failing osGreeterNoSession.config)
      );
      true
    )
    # Without scoot itself: refused the same way.
    (
      assert builtins.length (failing osGreeterNoEnable.config) == 1;
      true
    )
    (
      assert lib.hasInfix "needs programs.scoot.enable" (
        builtins.head (failing osGreeterNoEnable.config)
      );
      true
    )
    # With a backdrop beside Stylix's regreet image theming: refused
    # (two owners for one backdrop), naming both switches.
    (
      assert builtins.length (failing osGreeterStylix.config) == 1;
      true
    )
    (
      assert lib.hasInfix "programs.scoot.greeter.background is set" (
        builtins.head (failing osGreeterStylix.config)
      );
      true
    )
  ];
in
assert lib.all (x: x) _pins;
assert lib.all (x: x) _greeterPins;
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

  # 6. Session .desktop: runs the session launcher from the configured
  #    package, and names the desktop (which is what sets
  #    XDG_CURRENT_DESKTOP when launched from a display manager).
  grep -q "^Exec=${fakePkg}/bin/scoot-session$" ${desktopFile}
  grep -q '^DesktopNames=scoot$' ${desktopFile}
  grep -q '^Type=Application$' ${desktopFile}
  echo "ok: wayland-session entry runs the session launcher"

  # 6b. Default is byte-identical with the option present-but-null: the
  # exact-match `$` anchor above already proves no `-- COMMAND` suffix
  # (or anything else) leaked into the launcher entry.

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

  # 6f. The launcher's units: the service runs this package's binary on
  # `--tty`, bound to and before the session target it pulls in; the
  # shutdown target conflicts the session targets away (which is what
  # stops session-bound units when scoot exits). (These embed the unit
  # text in single quotes, so the resource files must stay free of
  # `'` -- keep the prose apostrophe-free.)
  printf '%s' '${sessionServiceText}' | grep -F -q "ExecStart=${fakePkg}/bin/scoot --tty"
  printf '%s' '${sessionServiceText}' | grep -F -x -q 'BindsTo=graphical-session.target'
  printf '%s' '${sessionServiceText}' | grep -F -x -q 'Before=graphical-session.target'
  printf '%s' '${sessionShutdownText}' | grep -F -x -q 'Conflicts=graphical-session.target graphical-session-pre.target'
  echo "ok: session user units wire the service to the session target"

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

  # 9. Stylix: the rendered file carries the themed appearance, cursor
  #    and wallpaper (with the injected scootbg `command` beside the
  #    Stylix image and mode), and nothing else.
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  want = {
      "appearance": {
          "focus_ring_active_color": "#0000ff",
          "focus_ring_inactive_color": "#303030",
          "background_color": "#101010",
          "cursor_theme": "Vanilla-DMZ",
          "cursor_size": 24,
      },
      "wallpaper": {
          "command": sys.argv[2],
          "image": sys.argv[3],
          "mode": "fill",
      },
  }
  assert got == want, f"{got} != {want}"
  ' ${hmStylixToml} '${fakeBg}/bin/scootbg' '${styImage}'
  echo "ok: Stylix defaults render (appearance, cursor, wallpaper + command)"

  touch $out
  echo "scoot-modules: all file-content checks passed"
''
