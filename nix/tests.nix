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
#   wiring: user-manager import, `scoot-session.target` reaching
#   `graphical-session.target` past the display import, the
#   activation environment, teardown), whose user units
#   (`scoot.service` with `PartOf` the session target and an
#   `ExecStart` naming the package's binary, `scoot-session.target`
#   `BindsTo` the graphical target it pulls in only once the display
#   is imported, plus `scoot-shutdown.target` conflicting all three
#   session targets away) are installed beside the entry and absent
#   with it off;
# - `session.command` renders verbatim into `Exec=` (launcher default,
#   `-- COMMAND` append, wrapper-script path, quoting
#   with spaces/quotes/pipes intact), stays inert with the entry off,
#   and refuses empty/blank and package-less combinations at eval;
# - the greeter (Linux only: it imports nixpkgs' own regreet module):
#   `greeter.enable` turns on `services.displayManager.regreet`, forces
#   the session entry (and its units) on beside it, confines cage to one
#   output (`-m last`, which a user's own `cageArgs` overrides), leaves
#   the backdrop alone by default and renders a set one as ReGreet's
#   `background.path`; off changes nothing; GDM/SDDM, an explicitly
#   disabled session entry, a missing `enable`, and a Stylix-owned
#   backdrop are each refused at eval, naming the conflict;
# - the two settings failure modes behave as documented (see below);
# - scootbg for `[wallpaper]` (ticket 10): the NixOS
#   `wallpaper.enable` follows `enable` and installs `wallpaper.package`,
#   off installs nothing, with no package it defaults off and only an
#   explicit `true` fails loudly at eval; the
#   home-manager side installs it whenever `settings.wallpaper` exists and
#   renders `command` as its store path (a user's own `command` wins); a
#   `{ url, hash }` image is fetched once at build time and renders as the
#   fetched file's store path (a set without both fails loudly), over a
#   `file://` fixture so the check needs no network;
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
#   `programs.scoot.stylix.wallpaper.enable = false` turns off just the
#   wallpaper pair (a user `color` then stands alone: the trap resolved),
#   keeping the appearance and cursor defaults;
#   the cursor package is never installed (Stylix's own cursor target
#   owns that); and the module evaluates with no Stylix option defined
#   at all (every pre-existing evaluation below does exactly that).
# - the desktop profile (`programs.scoot.desktop`, child
#   `desktop-profile`): `enable` turns on the session entry and its units
#   plus the wallpaper default on the NixOS side (staying additive: no
#   default session) and the portal config on the home-manager side, and
#   through `programs.scootbar` (when imported -- never required) the bar
#   with its unit; `look` renders the example palette into the compositor
#   and bar configs (each leaf yielding to a user value, and to Stylix
#   where present); a look without an in-repo wallpaper (`vinyl-sunset`)
#   sets no `[wallpaper]` table; the `[xwayland]` knob defaults on; the
#   idle policy (`desktop-idle-lock` child) runs with the profile -- dim
#   2 min to 10%, lock at 4, screens off at 5, lock-before-sleep, audio
#   hold -- each timeout overridable and each of the three switches
#   individually disable-able, the locker themed by the look unless
#   `theme.targets.lock.enable` opts out; an empty or quote-carrying
#   `lock.command`, and an out-of-range `dimLevel` while the dim step is
#   on, each fail eval; every remaining future slot
#   defaults off and inert; `enable` without scoot, a look without the
#   profile, and an unknown look each fail eval;
#   `desktop.greeter` is the greeter (the session entry forced on beside
#   it); and `bar.enable = false` leaves the bar entirely alone. The
#   profile and the policy are Linux-only: off Linux each tool defaults
#   to null, `lock.command` to its bare form, and the policy's own
#   assertions refuse loudly -- pinned, not skipped.
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
    # The desktop profile's bar half (`programs.scootbar` from
    # `nix/modules/scootbar-home.nix`) runs as a user service: only the
    # combined desktop evaluations below import that module, and this is
    # what its unit is pinned against.
    options.systemd.user.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
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
    # The bar's user service (`scootbar.service` from
    # `nix/modules/scootbar-nixos.nix`): only the combined desktop
    # evaluations below import that module.
    options.systemd.user.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    # The idle policy's docked-lid rule (the canonical
    # `settings.Login.HandleLidSwitchDocked` path, as far as the stub
    # goes: the real option is a freeform submodule, this proves the
    # value lands, and the real-NixOS pin below checks it against
    # nixpkgs' own module). Unset here, as there (logind's own
    # default, "ignore", then applies).
    options.services.logind.settings.Login = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
    };
    # The locker's PAM service (any key goes: the real option is an
    # attrs-of-submodule, this proves presence, not its schema).
    options.security.pam.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
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
  #
  # Plus what the scootbar module reads of it (the same rev:
  # `stylix/target.nix` for the palette, `stylix/fonts.nix` for the font
  # and size): `base05`/`base08` beside the three the compositor reads,
  # and the sans-serif family plus the desktop size. Only the combined
  # desktop-plus-bar evaluations below touch those; the compositor-only
  # pins above are unchanged by them.
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
      options.stylix.fonts.sansSerif = lib.mkOption {
        type = lib.types.nullOr lib.types.raw;
        default = null;
      };
      options.stylix.fonts.sizes.desktop = lib.mkOption {
        type = lib.types.int;
        default = 10;
      };
      config = {
        stylix.enable = enable;
        stylix.cursor = cursor;
        stylix.image = image;
        stylix.imageScalingMode = mode;
        stylix.fonts.sansSerif = {
          name = "DejaVu Sans";
          package = pkgs.dejavu_fonts.minimal;
        };
        stylix.fonts.sizes.desktop = 10;
        lib.stylix.colors.withHashtag = {
          base00 = "#101010";
          base03 = "#303030";
          base05 = "#e0e0e0";
          base08 = "#ff0000";
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
  # ...a `{ url, hash }` image: fetched once at build time (a `file://`
  # fixture, so the check needs no network: fixed-output fetching runs
  # anywhere), the settings and the rendered TOML naming the fetched file.
  urlWallpaperFile = builtins.toFile "url-wallpaper.png" "fake downloaded wallpaper";
  urlWallpaperHash = builtins.hashFile "sha256" urlWallpaperFile;
  hmWallUrl = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    settings.wallpaper = {
      image = {
        url = "file://${urlWallpaperFile}";
        hash = urlWallpaperHash;
      };
      mode = "fill";
    };
  };
  # ...a set without string `url` and `hash` fails loudly at eval (caught
  # here by `tryEval`, forcing the rendered config file).
  hmWallUrlBad =
    builtins.tryEval
      (evalHome {
        enable = true;
        settings.wallpaper.image = {
          url = "file://${urlWallpaperFile}";
        };
      }).config.xdg.configFile."scoot/config.toml".source;
  # ...and a per-output `{ url, hash }` image resolves the same way: the
  # top-level string renders as written beside the fetched per-output file.
  hmWallPerOutputUrl = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    settings.wallpaper = {
      image = "~/Pictures/hills.jpg";
      output."DP-2".image = {
        url = "file://${urlWallpaperFile}";
        hash = urlWallpaperHash;
      };
    };
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
  # The wallpaper defaults off (`stylix.wallpaper.enable = false`): no
  # `[wallpaper]` table added even with an image set (and so no scootbg),
  # while appearance and cursor stay Stylix's.
  hmStylixWallpaperOff = evalHomeStylix { } {
    enable = true;
    wallpaper.package = fakeBg;
    stylix.wallpaper.enable = false;
  };
  # The trap resolved: a user color with Stylix's image set, wallpaper
  # defaults off -- just the color, no image beside it.
  hmStylixColor = evalHomeStylix { } {
    enable = true;
    wallpaper.package = fakeBg;
    stylix.wallpaper.enable = false;
    settings.wallpaper.color = "#101014";
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
  # The desktop profile in a real NixOS evaluation: the idle policy's
  # system half against nixpkgs' own logind and PAM modules (not the
  # stubs above) -- in particular the canonical `settings.Login`
  # path, not the renamed alias.
  osRealIdle = evalRealNixos {
    enable = true;
    desktop.enable = true;
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
  # ...with the user's own `cageArgs`: kept verbatim (a plain assignment
  # beats the one-screen `mkDefault` below, which in turn beats nixpkgs'
  # option default).
  osGreeterCageOverride = evalNixosRegreet {
    enable = true;
    package = fakePkg;
    greeter.enable = true;
  } (greeterUser // { services.displayManager.regreet.cageArgs = [ "-s" ]; });

  # --- desktop profile (`programs.scoot.desktop`) evaluations under test ---
  #
  # The profile on its own (no scootbar module imported): the bar halves
  # stay empty (the profile never sets across the module boundary --
  # `home.nix` leaves `programs.scootbar` alone and `scootbar.nix` reads
  # the profile through `or {}`, defaulting absent), which is also
  # what proves the profile never requires the bar module.
  hmDesk = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
  };
  # One per look: the example palette as `[appearance]` plus, where the
  # look ships an in-repo wallpaper, its image and mode (which is what
  # installs scootbg for it); `vinyl-sunset` ships none, so no table
  # appears and the session shows the flat `background_color`.
  hmDeskLookMusic = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "music-desk";
  };
  hmDeskLookVinyl = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "vinyl-sunset";
  };
  hmDeskLookBurst = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "radial-burst";
  };
  hmDeskLookMoon = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "moonrise";
  };
  # A user value beside a look wins per key; the rest stays the look's.
  hmDeskUserWins = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "music-desk";
    settings.appearance.background_color = "#123456";
    settings.wallpaper.image = "/user/wall.png";
    settings.wallpaper.mode = "center";
  };
  # Stylix beside a look: Stylix wins every leaf (one priority above the
  # look), so this renders exactly what the Stylix-only evaluation does.
  hmDeskStylix = evalHomeStylix { } {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "music-desk";
  };
  # The `[xwayland]` knob: on means on, off means absent.
  hmDeskXwayland = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.xwayland.enable = true;
  };
  # A future slot enabled today: accepted and inert (assertions hold,
  # nothing installed beyond the profile's own).
  hmDeskSlotOn = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.notifications.enable = true;
  };

  # --- idle policy (`programs.scoot.desktop.idle`) evaluations ---
  #
  # The profile with a look: the whole policy on (swayidle unit, audio
  # inhibitor, locker config), themed by the look.
  hmIdle = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
  };
  # ...without a look: the policy runs unthemed (swaylock's own colors).
  hmIdleNoLook = evalHome {
    enable = true;
    desktop.enable = true;
  };
  # ...the policy off (each of the three switches back off: the
  # profile turns the set on, and the lock and inhibitor refuse to run
  # without it).
  hmIdleOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.idle.enable = false;
    desktop.idle.lock.enable = false;
    desktop.idle.mediaInhibit.enable = false;
  };
  # ...the lock off: dim and screens-off stay, nothing locks.
  hmLockOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.idle.lock.enable = false;
  };
  # ...the inhibitor off: the policy without audio hold.
  hmInhibitOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.idle.mediaInhibit.enable = false;
  };
  # ...retimed (each timeout and the dim level overridable, per the M2
  # reference they default from).
  hmIdleTimeouts = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.idle.dimTimeout = 60;
    desktop.idle.dimLevel = 20;
    desktop.idle.lockTimeout = 90;
    desktop.idle.offTimeout = 120;
  };
  # ...with steps disabled (0 omits that timeout's line).
  hmIdleZero = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.idle.dimTimeout = 0;
    desktop.idle.offTimeout = 0;
  };
  # ...with the lock action overridden (what a future `desktop-keys`
  # bind runs).
  hmLockCmd = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.idle.lock.command = "loginctl lock-session";
  };
  # ...with an empty lock action, and a quote-carrying one (each
  # refused at eval: the action renders inside single quotes on the
  # swayidle timeout line).
  hmLockCmdEmpty = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.idle.lock.command = "";
  };
  hmLockCmdQuote = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.idle.lock.command = "loginctl lock-session'; reboot";
  };
  # ...with the dim step off and an out-of-range level (unread, so no
  # refusal: the range applies only while the step is on).
  hmIdleZeroBadLevel = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.idle.dimTimeout = 0;
    desktop.idle.dimLevel = 0;
  };
  # ...with locker settings (a color winning per key, plus a bare
  # flag).
  hmLockSettings = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.idle.lock.settings = {
      ring-color = "#123456";
      show-failed-attempts = "";
    };
  };
  # ...opted out of locker theming (the look leaves the locker alone,
  # the settings still apply).
  hmThemeTargetOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.theme.targets.lock.enable = false;
    desktop.idle.lock.settings = {
      ring-color = "#123456";
    };
  };
  # ...standalone (no profile): the policy runs, unthemed.
  hmIdleStandalone = evalHome {
    enable = true;
    desktop.idle.enable = true;
  };
  # Refusals: the lock without the policy, and the inhibitor without it
  # (each pinned by message in `_idlePins`).
  hmLockNoIdle = evalHome {
    enable = true;
    desktop.idle.lock.enable = true;
  };
  hmInhibitNoIdle = evalHome {
    enable = true;
    desktop.idle.mediaInhibit.enable = true;
  };
  # ...the policy with no swayidle to run it.
  hmIdleNoPkg = evalHome {
    enable = true;
    desktop.idle.enable = true;
    desktop.idle.package = null;
  };
  # ...a dim level outside 1..100, and a negative timeout.
  hmIdleBadLevel = evalHome {
    enable = true;
    desktop.idle.enable = true;
    desktop.idle.dimLevel = 0;
  };
  hmIdleNeg = evalHome {
    enable = true;
    desktop.idle.enable = true;
    desktop.idle.lockTimeout = -1;
  };
  # Refusals: the profile without scoot, and a look without the profile
  # (both pinned by message in `_desktopPins`)...
  hmDeskNoEnable = evalHome { desktop.enable = true; };
  hmDeskLookNoEnable = evalHome {
    enable = true;
    desktop.look = "music-desk";
  };
  # ...and an unknown look, which is an option type error (the `enum`'s own
  # message names the valid values), caught here by `tryEval`.
  hmDeskUnknownLook =
    builtins.tryEval
      (evalHome {
        enable = true;
        desktop.enable = true;
        desktop.look = "bogus-look";
      }).config.programs.scoot.desktop.look;

  osDesk = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
  };
  osDeskSlotOn = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.notifications.enable = true;
  };
  # The knob is accepted here (its effect is home-manager wiring; the
  # package choice stays `programs.scoot.package`).
  osDeskXwayland = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.xwayland.enable = true;
  };
  osDeskNoEnable = evalNixos { desktop.enable = true; };
  osDeskLookNoEnable = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.look = "radial-burst";
  };
  # --- idle policy (`programs.scoot.desktop.idle`) system evaluations ---
  #
  # The profile: the tools installed, the docked-lid rule locking, the
  # locker's PAM service present, still additive (no default session).
  osIdle = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
  };
  # ...the policy off: the rule untouched, no PAM, the profile's own
  # packages only.
  osIdleOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.idle.enable = false;
    desktop.idle.lock.enable = false;
    desktop.idle.mediaInhibit.enable = false;
  };
  # ...the lock off: no PAM and no locker, the lid rule still locking
  # (dim and screens-off still run from the home-manager side).
  osLockOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.idle.lock.enable = false;
  };
  # ...with an explicit lid rule: the user's value wins over the
  # profile's docked-lock default (a separate module, the way the
  # greeter's overrides ride along -- the evaluation above can only
  # set `programs.scoot`).
  osIdleLidOverride =
    evalNixosWith
      [
        ./modules/nixos.nix
        { programs.scoot.wallpaper.package = lib.mkDefault fakeBg; }
        { services.logind.settings.Login.HandleLidSwitchDocked = "ignore"; }
      ]
      pkgs
      {
        enable = true;
        package = fakePkg;
        desktop.enable = true;
      };
  # Refusals: the lock without the policy (pinned by message), and the
  # policy with no swayidle to install.
  osLockNoIdle = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.idle.lock.enable = true;
  };
  osIdleNoPkg = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.idle.package = null;
  };
  # ...with an empty lock action, and a quote-carrying one (each
  # refused at eval on this side as well).
  osLockCmdEmpty = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.idle.lock.command = "";
  };
  osLockCmdQuote = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.idle.lock.command = "loginctl lock-session'; reboot";
  };
  # `desktop.greeter` is an alias for `programs.scoot.greeter`: through it
  # the profile lists a session in a ReGreet login.
  osDeskGreeter = evalNixosRegreet {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.greeter.enable = true;
  } greeterUser;

  # The profile beside the bar module: the bar halves have something to
  # set. A stand-in bar package (the pure nixpkgs set has no scootbar, and
  # a null one is refused by name).
  fakeBar = pkgs.runCommand "fake-scootbar" { } ''
    mkdir -p $out/bin
    echo '#!/bin/sh' > $out/bin/scootbar
    chmod +x $out/bin/scootbar
  '';
  evalHomeDesktopWith =
    extra: scootCfg: barCfg:
    lib.evalModules {
      modules = [
        ./modules/home.nix
        ./modules/scootbar-home.nix
        baseStubs
        homeStubs
        ({ config, ... }: {
          programs.scoot = scootCfg;
          programs.scootbar = barCfg;
        })
      ]
      ++ extra;
      specialArgs = { inherit pkgs; };
    };
  evalHomeDesktop = evalHomeDesktopWith [ ];
  evalNixosDesktopWith =
    extra: scootCfg: barCfg:
    lib.evalModules {
      modules = [
        ./modules/nixos.nix
        ./modules/scootbar-nixos.nix
        baseStubs
        nixosStubs
        regreetStubs
        { programs.scoot.wallpaper.package = lib.mkDefault fakeBg; }
        ({ config, ... }: {
          programs.scoot = scootCfg;
          programs.scootbar = barCfg;
        })
      ]
      ++ extra;
      specialArgs = {
        pkgs = pkgs;
        modulesPath = "${pkgs.path}/nixos/modules";
      };
    };
  evalNixosDesktop = evalNixosDesktopWith [ ];
  hmDeskBar = evalHomeDesktop {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "music-desk";
  } { package = fakeBar; };
  hmDeskBarUserWins =
    evalHomeDesktop
      {
        enable = true;
        package = fakePkg;
        wallpaper.package = fakeBg;
        desktop.enable = true;
        desktop.look = "music-desk";
      }
      {
        package = fakeBar;
        settings.colors.accent = "#123456";
      };
  hmDeskBarStylix = evalHomeDesktopWith [ (stylixStub { }) ] {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "music-desk";
  } { package = fakeBar; };
  hmDeskBarOff = evalHomeDesktop {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.bar.enable = false;
  } { package = fakeBar; };
  hmDeskBarMoon = evalHomeDesktop {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "moonrise";
  } { package = fakeBar; };
  osDeskBar = evalNixosDesktop {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.look = "radial-burst";
  } { package = fakeBar; };
  osDeskBarOff = evalNixosDesktop {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.look = "radial-burst";
    desktop.bar.enable = false;
  } { package = fakeBar; };

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

    # The launcher's units ride with the entry: all three installed...
    (
      assert osSession.config.systemd.user.units ? "scoot.service";
      true
    )
    (
      assert osSession.config.systemd.user.units ? "scoot-session.target";
      true
    )
    (
      assert osSession.config.systemd.user.units ? "scoot-shutdown.target";
      true
    )
    # ...the service bound to the session target it stops with, and to
    # no graphical target (starting the service must not pull the
    # session in: the launcher starts `scoot-session.target` past the
    # display import, and anything binding the graphical target at
    # fork time reintroduces the skipped-`ConditionEnvironment` bug)...
    (
      assert lib.hasInfix "PartOf=scoot-session.target" sessionServiceText;
      true
    )
    # ...and to no graphical target: only exact directive lines count
    # (the comments discuss the target they deliberately do not bind).
    (
      assert !(builtins.elem "BindsTo=graphical-session.target" sessionServiceLines);
      true
    )
    (
      assert !(builtins.elem "Before=graphical-session.target" sessionServiceLines);
      true
    )
    # ...the session target pulling in the graphical target once the
    # launcher starts it (a requirement dependency: starting the
    # session target starts the graphical one, which is how a
    # `RefuseManualStart=yes` target is reached at all)...
    (
      assert lib.hasInfix "BindsTo=graphical-session.target" sessionTargetText;
      true
    )
    # ...launching this package's binary on `--tty` (the same build the
    # entry names, wrapper included)...
    (
      assert contains "ExecStart=${fakePkg}/bin/scoot --tty" sessionServiceText;
      true
    )
    # ...and the shutdown target conflicting every session target away
    # (which is what stops the compositor through `PartOf`, and the
    # session-bound units through theirs, when scoot exits).
    (
      assert lib.hasInfix "Conflicts=scoot-session.target graphical-session.target" sessionShutdownText;
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
    # ...a `{ url, hash }` image is fetched at build time (which is what
    # turns `wallpaper.enable` on and installs scootbg for it; the
    # rendered TOML below names the fetched file, while `settings` keeps
    # the user's value verbatim until render)...
    (
      assert allAssertionsHold hmWallUrl.config;
      true
    )
    (
      assert hmWallUrl.config.programs.scoot.wallpaper.enable;
      true
    )
    # ...and a set without string `url` and `hash` never renders.
    (
      assert !hmWallUrlBad.success;
      true
    )
    # ...and a per-output `{ url, hash }` image is fetched at build time
    # too (the rendered TOML below names the fetched file).
    (
      assert allAssertionsHold hmWallPerOutputUrl.config;
      true
    )
    (
      assert hmWallPerOutputUrl.config.programs.scoot.wallpaper.enable;
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
    # `stylix.wallpaper.enable = false` turns off just the wallpaper
    # defaults: no `[wallpaper]` table added even with an image set, so
    # no scootbg is installed for it...
    (
      assert !(hmStylixWallpaperOff.config.programs.scoot.settings ? wallpaper);
      true
    )
    (
      assert !hmStylixWallpaperOff.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      assert hmStylixWallpaperOff.config.home.packages == [ ];
      true
    )
    # ...while the appearance and cursor defaults stay Stylix's.
    (
      assert
        hmStylixWallpaperOff.config.programs.scoot.settings.appearance
        == hmStylix.config.programs.scoot.settings.appearance;
      true
    )
    # The trap resolved: a user color with Stylix's image set and the
    # wallpaper defaults off is just the color (the injected scootbg
    # `command` joins it at render, as for any `[wallpaper]` table).
    (
      assert hmStylixColor.config.programs.scoot.settings.wallpaper == { color = "#101014"; };
      true
    )
    (
      assert hmStylixColor.config.programs.scoot.wallpaper.enable;
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
        assert osRealGreeter.config.systemd.user.units ? "scoot-session.target";
        # The one-screen cage default holds in a real NixOS evaluation
        # too, not only against the stubs.
        assert
          osRealGreeter.config.services.displayManager.regreet.cageArgs == [
            "-s"
            "-d"
            "-m"
            "last"
          ];
        true
      )
      # The profile's idle half in a real NixOS evaluation too: the
      # docked-lid rule on the canonical logind path, the locker's PAM
      # service, the tools installed -- and still no default session.
      # (No `allAssertionsHold`: a bare `eval-config.nix` always carries
      # the generic no-filesystem/no-bootloader failures, the way
      # `osRealGreeter` shows; the stubs above are where every
      # assertion is held.)
      (
        assert osRealIdle.config.services.logind.settings.Login.HandleLidSwitchDocked == "lock";
        assert osRealIdle.config.security.pam.services ? swaylock;
        assert osRealIdle.config.services.displayManager.defaultSession == null;
        assert lib.any (p: (p.pname or "") == "swayidle") osRealIdle.config.environment.systemPackages;
        assert lib.any (p: (p.pname or "") == "swaylock") osRealIdle.config.environment.systemPackages;
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
  sessionServiceLines = lib.splitString "\n" sessionServiceText;
  sessionTargetText = osSession.config.systemd.user.units."scoot-session.target".text;
  sessionShutdownText = osSession.config.systemd.user.units."scoot-shutdown.target".text;
  cmdDesktopFile = "${builtins.head osSessionCmd.config.services.displayManager.sessionPackages}/share/wayland-sessions/scoot.desktop";
  wrapperDesktopFile = "${builtins.head osSessionWrapper.config.services.displayManager.sessionPackages}/share/wayland-sessions/scoot.desktop";
  quotingDesktopFile = "${builtins.head osSessionQuoting.config.services.displayManager.sessionPackages}/share/wayland-sessions/scoot.desktop";
  hmWallToml = hmWall.config.xdg.configFile."scoot/config.toml".source;
  hmWallOwnCommandToml = hmWallOwnCommand.config.xdg.configFile."scoot/config.toml".source;
  hmWallOffToml = hmWallOff.config.xdg.configFile."scoot/config.toml".source;
  hmNoWallToml = hmNoWall.config.xdg.configFile."scoot/config.toml".source;
  hmWallNotTableToml = hmWallNotTable.config.xdg.configFile."scoot/config.toml".source;
  hmWallUrlToml = hmWallUrl.config.xdg.configFile."scoot/config.toml".source;
  hmWallPerOutputUrlToml = hmWallPerOutputUrl.config.xdg.configFile."scoot/config.toml".source;
  hmStylixToml = hmStylix.config.xdg.configFile."scoot/config.toml".source;
  hmStylixColorToml = hmStylixColor.config.xdg.configFile."scoot/config.toml".source;
  hmDeskMusicToml = hmDeskLookMusic.config.xdg.configFile."scoot/config.toml".source;
  hmDeskVinylToml = hmDeskLookVinyl.config.xdg.configFile."scoot/config.toml".source;
  hmDeskMoonToml = hmDeskLookMoon.config.xdg.configFile."scoot/config.toml".source;
  hmDeskBarToml = hmDeskBar.config.programs.scootbar.configFile;
  hmDeskBarMoonToml = hmDeskBarMoon.config.programs.scootbar.configFile;
  osDeskBarToml = osDeskBar.config.programs.scootbar.configFile;
  # The idle policy's generated files: the swayidle config (timeouts,
  # sleep lock, lock event) and the swaylock config (themed leaves),
  # plus the lock-off, zeroed, retimed, rebound, recolored, unthemed
  # and opt-out variants the content checks read.
  idleConf = hmIdle.config.xdg.configFile."swayidle/config".source;
  idleLockConf = hmIdle.config.xdg.configFile."swaylock/config".source;
  idleNoLockConf = hmLockOff.config.xdg.configFile."swayidle/config".source;
  idleTimeoutsConf = hmIdleTimeouts.config.xdg.configFile."swayidle/config".source;
  idleZeroConf = hmIdleZero.config.xdg.configFile."swayidle/config".source;
  idleCmdConf = hmLockCmd.config.xdg.configFile."swayidle/config".source;
  idleSettingsConf = hmLockSettings.config.xdg.configFile."swaylock/config".source;
  idleTargetOffConf = hmThemeTargetOff.config.xdg.configFile."swaylock/config".source;
  idleNoLookConf = hmIdleNoLook.config.xdg.configFile."swaylock/config".source;

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
      assert osGreeter.config.systemd.user.units ? "scoot-session.target";
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
    # One screen by default: cage confined to a single output, on top of
    # nixpkgs' spanning `[ "-s" "-d" ]` default (our `mkDefault` beats
    # the option default)...
    (
      assert
        osGreeter.config.services.displayManager.regreet.cageArgs == [
          "-s"
          "-d"
          "-m"
          "last"
        ];
      true
    )
    # ...while a user's own `cageArgs` wins over it (a plain assignment
    # beats our `mkDefault`).
    (
      assert allAssertionsHold osGreeterCageOverride.config;
      true
    )
    (
      assert osGreeterCageOverride.config.services.displayManager.regreet.cageArgs == [ "-s" ];
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

  # --- desktop profile structural pins (fail `nix flake check` at eval) ---
  # The desktop profile is Linux-only (user units, logind, seat
  # rights): off Linux its tools default to null and the policy's own
  # assertions refuse loudly (pinned in `_darwinIdlePins` below), so
  # these pins run only where the policy can run.
  _desktopPins = lib.optionals isLinux [
    # The profile alone (no bar module): assertions hold, portals on, and
    # nothing themed without a look...
    (
      assert allAssertionsHold hmDesk.config;
      true
    )
    (
      assert hmDesk.config.programs.scoot.portals.enable;
      true
    )
    (
      assert !(hmDesk.config.programs.scoot.settings ? appearance);
      true
    )
    (
      assert !(hmDesk.config.programs.scoot.settings ? wallpaper);
      true
    )
    # ...and the idle policy on with the profile (the
    # `desktop-idle-lock` child): swayidle, the dim and screens-off
    # tools, the locker and the audio inhibitor.
    (
      assert hmDesk.config.programs.scoot.desktop.idle.enable;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.idle.lock.enable;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.idle.mediaInhibit.enable;
      true
    )
    # ...the M2's timeouts as defaults (dim 2 min to 10%, lock at 4,
    # screens off at 5)...
    (
      assert hmDesk.config.programs.scoot.desktop.idle.dimTimeout == 120;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.idle.dimLevel == 10;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.idle.lockTimeout == 240;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.idle.offTimeout == 300;
      true
    )
    # ...the swaylock daemon behind the lock, and the locker theme
    # opt-out on (without forcing the half-built `theme` slot on)...
    (
      assert hmDesk.config.programs.scoot.desktop.idle.lock.daemon == "swaylock";
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.theme.targets.lock.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.theme.enable;
      true
    )
    # ...and every remaining future slot off and empty (spot-check
    # across the tree).
    (
      assert !hmDesk.config.programs.scoot.desktop.notifications.enable;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.notifications.package == null;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.launcher.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.capture.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.auth.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.secrets.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.audio.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.clipboard.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.nightlight.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.power.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.theme.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.apps.terminal.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.apps.fileManager.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.keys.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.displays.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.inputMethod.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.automount.enable;
      true
    )
    # music-desk: the example palette in the compositor config...
    (
      assert allAssertionsHold hmDeskLookMusic.config;
      true
    )
    (
      assert
        hmDeskLookMusic.config.programs.scoot.settings.appearance == {
          background_color = "#FCFBFB";
          focus_ring_active_color = "#3D579A";
          focus_ring_inactive_color = "#D5D7DD";
        };
      true
    )
    (
      assert hmDeskLookMusic.config.programs.scoot.settings.wallpaper.mode == "fill";
      true
    )
    (
      assert lib.hasSuffix "music-desk.png"
        hmDeskLookMusic.config.programs.scoot.settings.wallpaper.image;
      true
    )
    # ...which is what installs scootbg for it (beside the idle
    # policy's five tools, on with the profile).
    (
      assert hmDeskLookMusic.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      assert
        sorted hmDeskLookMusic.config.home.packages == sorted [
          fakePkg
          fakeBg
          pkgs.swayidle
          pkgs.brightnessctl
          pkgs.wlopm
          pkgs.swaylock
          pkgs.sway-audio-idle-inhibit
        ];
      true
    )
    # vinyl-sunset: colors but no wallpaper table (the illustration cannot
    # be committed), so no scootbg for it -- the flat background_color shows.
    (
      assert
        hmDeskLookVinyl.config.programs.scoot.settings.appearance == {
          background_color = "#271A1F";
          focus_ring_active_color = "#E59560";
          focus_ring_inactive_color = "#423F51";
        };
      true
    )
    (
      assert !(hmDeskLookVinyl.config.programs.scoot.settings ? wallpaper);
      true
    )
    (
      assert !hmDeskLookVinyl.config.programs.scoot.wallpaper.enable;
      true
    )
    # radial-burst: colors plus its shipped image.
    (
      assert
        hmDeskLookBurst.config.programs.scoot.settings.appearance == {
          background_color = "#241721";
          focus_ring_active_color = "#31a9e5";
          focus_ring_inactive_color = "#e36e38";
        };
      true
    )
    (
      assert lib.hasSuffix "radial-burst.png"
        hmDeskLookBurst.config.programs.scoot.settings.wallpaper.image;
      true
    )
    # moonrise: colors plus its shipped image.
    (
      assert
        hmDeskLookMoon.config.programs.scoot.settings.appearance == {
          background_color = "#2B3648";
          focus_ring_active_color = "#FF9A49";
          focus_ring_inactive_color = "#5E4B5B";
        };
      true
    )
    (
      assert hmDeskLookMoon.config.programs.scoot.settings.wallpaper.mode == "fill";
      true
    )
    (
      assert lib.hasSuffix "moonrise.png" hmDeskLookMoon.config.programs.scoot.settings.wallpaper.image;
      true
    )
    # ...which is what installs scootbg for it.
    (
      assert hmDeskLookMoon.config.programs.scoot.wallpaper.enable;
      true
    )
    # A user value beside a look wins per key; the rest stays the look's.
    (
      assert hmDeskUserWins.config.programs.scoot.settings.appearance.background_color == "#123456";
      true
    )
    (
      assert
        hmDeskUserWins.config.programs.scoot.settings.appearance.focus_ring_active_color == "#3D579A";
      true
    )
    (
      assert hmDeskUserWins.config.programs.scoot.settings.wallpaper.image == "/user/wall.png";
      true
    )
    (
      assert hmDeskUserWins.config.programs.scoot.settings.wallpaper.mode == "center";
      true
    )
    # Stylix beside a look wins every leaf: identical to Stylix alone.
    (
      assert hmDeskStylix.config.programs.scoot.settings == hmStylix.config.programs.scoot.settings;
      true
    )
    # The xwayland knob defaults the compositor flag on...
    (
      assert hmDeskXwayland.config.programs.scoot.settings.xwayland.enabled;
      true
    )
    # ...and a future slot on is accepted and inert.
    (
      assert allAssertionsHold hmDeskSlotOn.config;
      true
    )
    (
      assert drvs hmDeskSlotOn.config.home.packages == drvs hmDesk.config.home.packages;
      true
    )
    # Refusals: the profile without scoot...
    (
      assert builtins.length (failing hmDeskNoEnable.config) == 1;
      true
    )
    (
      assert lib.hasInfix "desktop.enable needs" (builtins.head (failing hmDeskNoEnable.config));
      true
    )
    # ...a look without the profile...
    (
      assert builtins.length (failing hmDeskLookNoEnable.config) == 1;
      true
    )
    (
      assert lib.hasInfix "desktop.look needs" (builtins.head (failing hmDeskLookNoEnable.config));
      true
    )
    # ...and an unknown look, an enum type error (verified by hand to name
    # the four valid values).
    (
      assert !hmDeskUnknownLook.success;
      true
    )
    # NixOS: the profile turns on the session entry and its units, the
    # wallpaper default, and stays additive (no default session, ever)...
    (
      assert allAssertionsHold osDesk.config;
      true
    )
    (
      assert builtins.length osDesk.config.services.displayManager.sessionPackages == 1;
      true
    )
    (
      assert osDesk.config.systemd.user.units ? "scoot.service";
      true
    )
    (
      assert osDesk.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      assert osDesk.config.services.displayManager.defaultSession == null;
      true
    )
    # ...a future slot on is accepted and inert there too...
    (
      assert allAssertionsHold osDeskSlotOn.config;
      true
    )
    (
      assert
        drvs osDeskSlotOn.config.environment.systemPackages
        == drvs osDesk.config.environment.systemPackages;
      true
    )
    # ...the xwayland knob is accepted (its effect is home-manager wiring;
    # the package choice stays `programs.scoot.package`)...
    (
      assert allAssertionsHold osDeskXwayland.config;
      true
    )
    # ...and the refusals name the profile on this side as well.
    (
      assert lib.any (m: lib.hasInfix "desktop.enable needs" m) (failing osDeskNoEnable.config);
      true
    )
    (
      assert lib.any (m: lib.hasInfix "desktop.look needs" m) (failing osDeskLookNoEnable.config);
      true
    )
    # `desktop.greeter` is the greeter: through it the profile lists a
    # session in a ReGreet login (entry forced on beside it).
    (
      assert allAssertionsHold osDeskGreeter.config;
      true
    )
    (
      assert osDeskGreeter.config.services.displayManager.regreet.enable;
      true
    )
    (
      assert builtins.length osDeskGreeter.config.services.displayManager.sessionPackages == 1;
      true
    )
    # With the bar module: enable turns the bar on with the look's colors
    # and its unit...
    (
      assert allAssertionsHold hmDeskBar.config;
      true
    )
    (
      assert hmDeskBar.config.programs.scootbar.enable;
      true
    )
    (
      assert hmDeskBar.config.systemd.user.services ? scootbar;
      true
    )
    (
      assert
        hmDeskBar.config.programs.scootbar.settings.colors == {
          background = "#FCFBFB";
          foreground = "#1A2032";
          accent = "#3D579A";
          hover = "#5D7AB0";
          dim = "#C9CBD0";
          urgent = "#EE6F5E";
        };
      true
    )
    # ...and the moonrise look themes the bar the same way.
    (
      assert
        hmDeskBarMoon.config.programs.scootbar.settings.colors == {
          background = "#2B3648";
          foreground = "#F6EEDC";
          accent = "#FFA45C";
          hover = "#FFD54A";
          dim = "#9C8B95";
          urgent = "#E87F6A";
        };
      true
    )
    # ...a user color wins per key there too...
    (
      assert hmDeskBarUserWins.config.programs.scootbar.settings.colors.accent == "#123456";
      true
    )
    (
      assert hmDeskBarUserWins.config.programs.scootbar.settings.colors.background == "#FCFBFB";
      true
    )
    # ...Stylix wins every bar leaf as well...
    (
      assert
        hmDeskBarStylix.config.programs.scootbar.settings.colors == {
          background = "#101010";
          foreground = "#e0e0e0";
          accent = "#0000ff";
          hover = "#0000ff";
          dim = "#303030";
          urgent = "#ff0000";
        };
      true
    )
    (
      assert hmDeskBarStylix.config.programs.scootbar.settings.bar."font-size" == 13;
      true
    )
    (
      assert lib.hasPrefix builtins.storeDir hmDeskBarStylix.config.programs.scootbar.settings.bar.font;
      true
    )
    (
      assert hmDeskBarStylix.config.programs.scoot.settings == hmStylix.config.programs.scoot.settings;
      true
    )
    # ...and `bar.enable = false` leaves the bar entirely alone (no
    # bar service beside the profile's idle ones).
    (
      assert !hmDeskBarOff.config.programs.scootbar.enable;
      true
    )
    # ...and no look colors reach it either (no `colors` at all).
    (
      assert (hmDeskBarOff.config.programs.scootbar.settings.colors or { }) == { };
      true
    )
    (
      assert !(hmDeskBarOff.config.systemd.user.services ? scootbar);
      true
    )
    # ...including unthemed: no look color reaches a bar the profile
    # does not manage.
    (
      assert (hmDeskBarOff.config.programs.scootbar.settings.colors or { }) == { };
      true
    )
    # NixOS with the bar module: the unit and the /etc file beside the
    # session entry, in the look's colors...
    (
      assert allAssertionsHold osDeskBar.config;
      true
    )
    (
      assert osDeskBar.config.programs.scootbar.enable;
      true
    )
    (
      assert osDeskBar.config.systemd.user.services ? scootbar;
      true
    )
    (
      assert osDeskBar.config.environment.etc ? "scootbar/bar.toml";
      true
    )
    (
      assert
        osDeskBar.config.programs.scootbar.settings.colors == {
          background = "#241721";
          foreground = "#fdef1d";
          accent = "#31a9e5";
          dim = "#99911d";
          urgent = "#bf128d";
        };
      true
    )
    (
      assert osDeskBar.config.services.displayManager.defaultSession == null;
      true
    )
    (
      assert !osDeskBarOff.config.programs.scootbar.enable;
      true
    )
    (
      assert (osDeskBarOff.config.programs.scootbar.settings.colors or { }) == { };
      true
    )
    (
      assert osDeskBarOff.config.systemd.user.services == { };
      true
    )
    (
      assert (osDeskBarOff.config.programs.scootbar.settings.colors or { }) == { };
      true
    )
  ];

  # --- idle policy structural pins (fail `nix flake check` at eval) ---
  # Linux only, like the profile above: every evaluation here runs the
  # policy, whose tools refuse evaluation on Darwin (the null
  # degradation itself is pinned in `_darwinIdlePins`).
  _idlePins = lib.optionals isLinux [
    # Home-manager: the whole policy on (units, files, tools beside
    # the profile's own)...
    (
      assert allAssertionsHold hmIdle.config;
      true
    )
    (
      assert hmIdle.config.systemd.user.services ? scoot-idle;
      true
    )
    (
      assert hmIdle.config.systemd.user.services ? scoot-audio-inhibit;
      true
    )
    (
      assert hmIdle.config.xdg.configFile ? "swayidle/config";
      true
    )
    (
      assert hmIdle.config.xdg.configFile ? "swaylock/config";
      true
    )
    # ...bound to the graphical session (which the launcher reaches
    # past the display import), retried rather than conditioned...
    (
      assert
        hmIdle.config.systemd.user.services.scoot-idle.Install.WantedBy == [ "graphical-session.target" ];
      true
    )
    (
      assert hmIdle.config.systemd.user.services.scoot-idle.Unit.PartOf == [ "graphical-session.target" ];
      true
    )
    (
      assert hmIdle.config.systemd.user.services.scoot-idle.Unit.After == [ "graphical-session.target" ];
      true
    )
    # ...waiting for each command (the before-sleep lock lands before
    # logind sleeps) from the generated config...
    (
      assert lib.hasInfix "/bin/swayidle -w -C "
        hmIdle.config.systemd.user.services.scoot-idle.Service.ExecStart;
      true
    )
    # ...exactly the five tools installed (swayidle, dim, off, locker,
    # inhibitor -- no scoot package set here, so nothing else).
    (
      assert builtins.length hmIdle.config.home.packages == 5;
      true
    )
    (
      assert
        sorted hmIdle.config.home.packages == sorted [
          pkgs.swayidle
          pkgs.brightnessctl
          pkgs.wlopm
          pkgs.swaylock
          pkgs.sway-audio-idle-inhibit
        ];
      true
    )
    # Without a look the policy runs unthemed (the files and units are
    # still there; the locker keeps swaylock's own colors -- pinned by
    # content below).
    (
      assert allAssertionsHold hmIdleNoLook.config;
      true
    )
    (
      assert hmIdleNoLook.config.systemd.user.services ? scoot-idle;
      true
    )
    (
      assert hmIdleNoLook.config.xdg.configFile ? "swaylock/config";
      true
    )
    # The policy off: no units, no files beyond the profile's own, no
    # tools.
    (
      assert allAssertionsHold hmIdleOff.config;
      true
    )
    (
      assert hmIdleOff.config.systemd.user.services == { };
      true
    )
    (
      assert !(hmIdleOff.config.xdg.configFile ? "swayidle/config");
      true
    )
    (
      assert !(hmIdleOff.config.xdg.configFile ? "swaylock/config");
      true
    )
    (
      assert hmIdleOff.config.home.packages == [ ];
      true
    )
    # The lock off: the policy stays (dim and screens-off), the locker
    # leaves (no config, no package, four tools left).
    (
      assert allAssertionsHold hmLockOff.config;
      true
    )
    (
      assert hmLockOff.config.systemd.user.services ? scoot-idle;
      true
    )
    (
      assert !(hmLockOff.config.xdg.configFile ? "swaylock/config");
      true
    )
    (
      assert builtins.length hmLockOff.config.home.packages == 4;
      true
    )
    # The inhibitor off: the policy without the audio hold.
    (
      assert allAssertionsHold hmInhibitOff.config;
      true
    )
    (
      assert !(hmInhibitOff.config.systemd.user.services ? scoot-audio-inhibit);
      true
    )
    (
      assert builtins.length hmInhibitOff.config.home.packages == 4;
      true
    )
    # Retimed, zeroed, rebound and recolored: every assertion still
    # holds (the content checks below prove the values land).
    (
      assert allAssertionsHold hmIdleTimeouts.config;
      true
    )
    (
      assert allAssertionsHold hmIdleZero.config;
      true
    )
    (
      assert allAssertionsHold hmLockCmd.config;
      true
    )
    (
      assert allAssertionsHold hmLockSettings.config;
      true
    )
    (
      assert allAssertionsHold hmThemeTargetOff.config;
      true
    )
    # Standalone (no profile): the policy runs, unthemed and unlocked
    # (the locker's opt-in stays off with it).
    (
      assert allAssertionsHold hmIdleStandalone.config;
      true
    )
    (
      assert hmIdleStandalone.config.systemd.user.services ? scoot-idle;
      true
    )
    (
      assert !(hmIdleStandalone.config.xdg.configFile ? "swaylock/config");
      true
    )
    # Refusals: the lock without the policy...
    (
      assert builtins.length (failing hmLockNoIdle.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.lock.enable needs" (builtins.head (failing hmLockNoIdle.config));
      true
    )
    # ...the inhibitor without it...
    (
      assert builtins.length (failing hmInhibitNoIdle.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.mediaInhibit.enable needs" (
        builtins.head (failing hmInhibitNoIdle.config)
      );
      true
    )
    # ...the policy with no swayidle to run it...
    (
      assert builtins.length (failing hmIdleNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.package is null" (builtins.head (failing hmIdleNoPkg.config));
      true
    )
    # ...a dim level outside 1..100, and a negative timeout.
    (
      assert builtins.length (failing hmIdleBadLevel.config) == 1;
      true
    )
    (
      assert lib.hasInfix "dimLevel" (builtins.head (failing hmIdleBadLevel.config));
      true
    )
    (
      assert builtins.length (failing hmIdleNeg.config) == 1;
      true
    )
    # ...an out-of-range dim level with the dim step off (the level is
    # unread then, so the range does not fire)...
    (
      assert allAssertionsHold hmIdleZeroBadLevel.config;
      true
    )
    # ...an empty lock action, and a quote-carrying one (each refused
    # naming the command)...
    (
      assert builtins.length (failing hmLockCmdEmpty.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.lock.command" (builtins.head (failing hmLockCmdEmpty.config));
      true
    )
    (
      assert builtins.length (failing hmLockCmdQuote.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.lock.command" (builtins.head (failing hmLockCmdQuote.config));
      true
    )

    # NixOS: the profile installs the five tools beside scoot and
    # scootbg, locks docked lids, and names the locker's PAM service --
    # staying additive (no default session, ever)...
    (
      assert allAssertionsHold osIdle.config;
      true
    )
    (
      assert
        sorted osIdle.config.environment.systemPackages == sorted [
          fakePkg
          fakeBg
          pkgs.swayidle
          pkgs.brightnessctl
          pkgs.wlopm
          pkgs.swaylock
          pkgs.sway-audio-idle-inhibit
        ];
      true
    )
    (
      assert osIdle.config.services.logind.settings.Login.HandleLidSwitchDocked == "lock";
      true
    )
    (
      assert osIdle.config.security.pam.services ? swaylock;
      true
    )
    (
      assert osIdle.config.services.displayManager.defaultSession == null;
      true
    )
    # ...the policy off: the rule untouched (logind's own default
    # applies), no PAM, the profile's own packages only...
    (
      assert allAssertionsHold osIdleOff.config;
      true
    )
    (
      assert osIdleOff.config.services.logind.settings.Login == { };
      true
    )
    (
      assert !(osIdleOff.config.security.pam.services ? swaylock);
      true
    )
    (
      assert
        sorted osIdleOff.config.environment.systemPackages == sorted [
          fakePkg
          fakeBg
        ];
      true
    )
    # ...the lock off: no PAM and no locker, the lid rule still
    # locking (dim and screens-off still run from the home-manager
    # side)...
    (
      assert allAssertionsHold osLockOff.config;
      true
    )
    (
      assert !(osLockOff.config.security.pam.services ? swaylock);
      true
    )
    (
      assert
        sorted osLockOff.config.environment.systemPackages == sorted [
          fakePkg
          fakeBg
          pkgs.swayidle
          pkgs.brightnessctl
          pkgs.wlopm
          pkgs.sway-audio-idle-inhibit
        ];
      true
    )
    (
      assert osLockOff.config.services.logind.settings.Login.HandleLidSwitchDocked == "lock";
      true
    )
    # ...an explicit lid rule winning over the profile's default...
    (
      assert allAssertionsHold osIdleLidOverride.config;
      true
    )
    (
      assert osIdleLidOverride.config.services.logind.settings.Login.HandleLidSwitchDocked == "ignore";
      true
    )
    # ...and the refusals naming the policy on this side as well.
    (
      assert builtins.length (failing osLockNoIdle.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.lock.enable needs" (builtins.head (failing osLockNoIdle.config));
      true
    )
    (
      assert builtins.length (failing osIdleNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.package is null" (builtins.head (failing osIdleNoPkg.config));
      true
    )
    # ...and an empty lock action, and a quote-carrying one (each
    # refused naming the command on this side as well).
    (
      assert builtins.length (failing osLockCmdEmpty.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.lock.command" (builtins.head (failing osLockCmdEmpty.config));
      true
    )
    (
      assert builtins.length (failing osLockCmdQuote.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.lock.command" (builtins.head (failing osLockCmdQuote.config));
      true
    )
  ];

  # --- idle policy off Linux (fail `nix flake check` at eval) ---
  #
  # The tools above are Linux-only: off Linux each package defaults to
  # null (their attributes exist on Darwin but refuse evaluation when
  # forced, so `or null` alone does not save them), the lock action
  # falls back to its bare form, and the policy's own assertions refuse
  # loudly instead of installing nothing silently. Empty off Linux (the
  # Linux check above is where the policy is pinned).
  _darwinIdlePins = lib.optionals (!isLinux) [
    # Home-manager: every tool null, nothing installed for the policy...
    (
      assert hmIdle.config.programs.scoot.desktop.idle.package == null;
      true
    )
    (
      assert hmIdle.config.programs.scoot.desktop.idle.dimPackage == null;
      true
    )
    (
      assert hmIdle.config.programs.scoot.desktop.idle.offPackage == null;
      true
    )
    (
      assert hmIdle.config.programs.scoot.desktop.idle.mediaInhibit.package == null;
      true
    )
    (
      assert hmIdle.config.programs.scoot.desktop.idle.lock.package == null;
      true
    )
    # ...the lock action in its bare form (no store path: logind is
    # Linux-only)...
    (
      assert hmIdle.config.programs.scoot.desktop.idle.lock.command == "loginctl lock-session";
      true
    )
    # ...and the policy's own assertions refusing loudly, naming the
    # switch (one per null tool: the policy, the dim and screens-off
    # steps, the inhibitor, the locker).
    (
      assert builtins.length (failing hmIdle.config) == 5;
      true
    )
    (
      assert lib.hasInfix "idle.package is null" (builtins.head (failing hmIdle.config));
      true
    )
    # NixOS: the same nulls (no tools installed for the policy)...
    (
      assert osIdle.config.programs.scoot.desktop.idle.package == null;
      true
    )
    (
      assert osIdle.config.programs.scoot.desktop.idle.lock.package == null;
      true
    )
    (
      assert osIdle.config.programs.scoot.desktop.idle.lock.command == "loginctl lock-session";
      true
    )
    # ...refused loudly there too, while the docked-lid rule (plain
    # values, no tools) still lands.
    (
      assert builtins.length (failing osIdle.config) == 5;
      true
    )
    (
      assert osIdle.config.services.logind.settings.Login.HandleLidSwitchDocked == "lock";
      true
    )
  ];
in
assert lib.all (x: x) _pins;
assert lib.all (x: x) _greeterPins;
assert lib.all (x: x) _desktopPins;
assert lib.all (x: x) _idlePins;
assert lib.all (x: x) _darwinIdlePins;
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
  # `--tty`, stopped with the session target it is `PartOf` (and bound
  # to no graphical target, so starting the service never reaches the
  # session early); the session target pulls the graphical target in
  # once the launcher starts it past the display import; the shutdown
  # target conflicts every session target away (stopping the
  # compositor through `PartOf` when scoot exits). (These embed the
  # unit text in single quotes, so the resource files must stay free
  # of `'` -- keep the prose apostrophe-free.)
  printf '%s' '${sessionServiceText}' | grep -F -q "ExecStart=${fakePkg}/bin/scoot --tty"
  printf '%s' '${sessionServiceText}' | grep -F -x -q 'PartOf=scoot-session.target'
  if printf '%s' '${sessionServiceText}' | grep -F -x -q -e 'BindsTo=graphical-session.target' -e 'Before=graphical-session.target'; then echo "scoot.service must not bind the graphical target" >&2; exit 1; fi
  printf '%s' '${sessionTargetText}' | grep -F -x -q 'BindsTo=graphical-session.target'
  printf '%s' '${sessionShutdownText}' | grep -F -x -q 'Conflicts=scoot-session.target graphical-session.target graphical-session-pre.target'
  echo "ok: session user units reach the session target past the display import"

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
  #    ...a `{ url, hash }` image renders as the fetched file's store path
  #    (beside the injected scootbg `command`).
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))["wallpaper"]
  assert got["mode"] == "fill", got
  assert got["image"].endswith("url-wallpaper.png"), got
  assert got["command"].endswith("/bin/scootbg"), got
  ' ${hmWallUrlToml}
  echo "ok: a [wallpaper] { url, hash } image renders as the fetched file"
  #    ...and a per-output `{ url, hash }` image renders as the fetched
  #    file's store path beside the top-level image as written.
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))["wallpaper"]
  assert got["image"] == "~/Pictures/hills.jpg", got
  per = got["output"]["DP-2"]
  assert per["image"].endswith("url-wallpaper.png"), got
  assert got["command"].endswith("/bin/scootbg"), got
  ' ${hmWallPerOutputUrlToml}
  echo "ok: a per-output [wallpaper] { url, hash } image renders as the fetched file"

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

  # 9b. Stylix wallpaper defaults off with a user color: just the color
  #    (and the injected `command`), no Stylix image beside it -- while
  #    the themed appearance renders as before.
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  assert got["wallpaper"] == {"color": "#101014", "command": sys.argv[2]}, got["wallpaper"]
  assert got["appearance"]["background_color"] == "#101010", got["appearance"]
  ' ${hmStylixColorToml} '${fakeBg}/bin/scootbg'
  echo "ok: stylix.wallpaper.enable = false leaves a user color alone"

  # 10. Desktop profile, music-desk: the example palette in the compositor
  #     config (appearance plus the shipped wallpaper beside the injected
  #     scootbg command), and the same palette in the bar file.
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  assert got["appearance"] == {"background_color": "#FCFBFB", "focus_ring_active_color": "#3D579A", "focus_ring_inactive_color": "#D5D7DD"}, got["appearance"]
  assert got["wallpaper"]["mode"] == "fill", got["wallpaper"]
  assert got["wallpaper"]["image"].endswith("music-desk.png"), got["wallpaper"]
  assert got["wallpaper"]["command"].endswith("/bin/scootbg"), got["wallpaper"]
  ' ${hmDeskMusicToml}
  echo "ok: desktop look renders the example palette plus its wallpaper"
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  assert got["colors"] == {"background": "#FCFBFB", "foreground": "#1A2032", "accent": "#3D579A", "hover": "#5D7AB0", "dim": "#C9CBD0", "urgent": "#EE6F5E"}, got["colors"]
  ' ${hmDeskBarToml}
  echo "ok: desktop look renders the example palette into the bar config"

  # 10b. Desktop profile, vinyl-sunset: colors but no wallpaper table (the
  #      illustration cannot be committed -- the flat background_color is
  #      the session's flat color).
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  assert got["appearance"]["background_color"] == "#271A1F", got["appearance"]
  assert "wallpaper" not in got, got
  ' ${hmDeskVinylToml}
  echo "ok: a look without an in-repo wallpaper sets no wallpaper table"

  # 10c. Desktop profile, radial-burst on the NixOS side: the bar file
  #      carries that look's colors (five tokens: no hover there).
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  assert got["colors"] == {"background": "#241721", "foreground": "#fdef1d", "accent": "#31a9e5", "dim": "#99911d", "urgent": "#bf128d"}, got["colors"]
  ' ${osDeskBarToml}
  echo "ok: desktop look renders into the NixOS-side bar config"

  # 10d. Desktop profile, moonrise: the example palette in the compositor
  #      config (appearance plus the shipped wallpaper beside the injected
  #      scootbg command), and the same palette in the bar file.
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  assert got["appearance"] == {"background_color": "#2B3648", "focus_ring_active_color": "#FF9A49", "focus_ring_inactive_color": "#5E4B5B"}, got["appearance"]
  assert got["wallpaper"]["mode"] == "fill", got["wallpaper"]
  assert got["wallpaper"]["image"].endswith("moonrise.png"), got["wallpaper"]
  assert got["wallpaper"]["command"].endswith("/bin/scootbg"), got["wallpaper"]
  ' ${hmDeskMoonToml}
  echo "ok: desktop look renders moonrise plus its wallpaper"
  python3 -c '
  import sys,tomllib
  got = tomllib.load(open(sys.argv[1],"rb"))
  assert got["colors"] == {"background": "#2B3648", "foreground": "#F6EEDC", "accent": "#FFA45C", "hover": "#FFD54A", "dim": "#9C8B95", "urgent": "#E87F6A"}, got["colors"]
  ' ${hmDeskBarMoonToml}
  echo "ok: desktop look renders moonrise into the bar config"

  # 11-12c. Idle policy and locker content (Linux only: every line
  # below names a Linux-only tool's store path, and the files
  # themselves exist only where the policy runs -- off Linux the
  # tools default to null and no config is written).
  ${lib.optionalString isLinux ''
    # 11. Idle policy: the generated swayidle config carries the M2's
    #     timeouts -- dim at 2 min with save/restore, lock at 4 min
    #     through loginctl, screens off at 5 min with the output wildcard
    #     quoted intact -- plus the sleep lock and the lock event behind
    #     `swaylock -f` with its config. (Fixed-string matches throughout:
    #     the quoting is the assertion.)
    grep -F "timeout 120 '${pkgs.brightnessctl}/bin/brightnessctl -s set 10%' resume '${pkgs.brightnessctl}/bin/brightnessctl -r'" ${idleConf}
    grep -F "timeout 240 '${pkgs.systemd}/bin/loginctl lock-session'" ${idleConf}
    grep -F "timeout 300 '${pkgs.wlopm}/bin/wlopm --off \"*\"' resume '${pkgs.wlopm}/bin/wlopm --on \"*\"'" ${idleConf}
    grep -F "before-sleep '${pkgs.swaylock}/bin/swaylock -f -C /nix/store/" ${idleConf}
    grep -F "lock '${pkgs.swaylock}/bin/swaylock -f -C /nix/store/" ${idleConf}
    echo "ok: swayidle config carries the idle timeouts, the sleep lock and the lock event"

    # 11b. The lock off: dim and screens-off stay, and no line locks
    #      (no timeout through loginctl, no before-sleep, no lock event).
    grep -F "timeout 120 '" ${idleNoLockConf}
    grep -F "timeout 300 '" ${idleNoLockConf}
    if grep -q lock ${idleNoLockConf}; then echo "lock lines present with the locker off" >&2; exit 1; fi
    echo "ok: with the lock off, dim and screens-off stay and nothing locks"

    # 11c. Retimed: every override lands (timeouts and the dim level).
    grep -F "timeout 60 '${pkgs.brightnessctl}/bin/brightnessctl -s set 20%'" ${idleTimeoutsConf}
    grep -F "timeout 90 '" ${idleTimeoutsConf}
    grep -F "timeout 120 '" ${idleTimeoutsConf}
    echo "ok: retimed idle policy renders its overrides"

    # 11d. Zeroed: a 0 timeout omits that step's line (the lock step
    #      stays: sleep still locks while the locker is on).
    if grep -q "timeout 120\|timeout 300" ${idleZeroConf}; then echo "disabled step still present" >&2; exit 1; fi
    grep -F "timeout 240 '" ${idleZeroConf}
    grep -F "before-sleep '" ${idleZeroConf}
    echo "ok: a 0 timeout omits that step"

    # 11e. Rebound: the lock action override is what the timeout runs
    #      (the future desktop-keys bind target).
    grep -F "timeout 240 'loginctl lock-session'" ${idleCmdConf}
    echo "ok: the lock action override reaches the timeout"

    # 12. Locker config, music-desk: the look's roles as swaylock leaves
    #     (screen and indicator backgrounds, active ring, accent key
    #     highlight, ink text, urgent wrong-ring).
    grep -F -x "color=FCFBFB" ${idleLockConf}
    grep -F -x "inside-color=FCFBFB" ${idleLockConf}
    grep -F -x "ring-color=3D579A" ${idleLockConf}
    grep -F -x "key-hl-color=3D579A" ${idleLockConf}
    grep -F -x "text-color=1A2032" ${idleLockConf}
    grep -F -x "ring-wrong-color=EE6F5E" ${idleLockConf}
    echo "ok: locker config carries the look's colors"

    # 12b. A locker setting wins per key (verbatim, leading `#` kept --
    #      swaylock's own parser strips it), and an empty string renders
    #      a bare flag.
    grep -F -x "ring-color=#123456" ${idleSettingsConf}
    grep -F -x "show-failed-attempts" ${idleSettingsConf}
    grep -F -x "color=FCFBFB" ${idleSettingsConf}
    echo "ok: locker settings win per key, flags render bare"

    # 12c. Opted out (or lookless): no themed leaf at all -- with the
    #      opt-out the settings still apply, so the locker is theirs.
    if grep -q "^color=" ${idleTargetOffConf}; then echo "themed leaf present with theming off" >&2; exit 1; fi
    grep -F -x "ring-color=#123456" ${idleTargetOffConf}
    if grep -q "color=" ${idleNoLookConf}; then echo "themed leaf present with no look" >&2; exit 1; fi
    echo "ok: opting out (or no look) leaves the locker unthemed"
  ''}

  touch $out
  echo "scoot-modules: all file-content checks passed"
''
