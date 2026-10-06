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
#   output (`-m last`, which a user's own `cageArgs` overrides), ends the
#   greeter's processes with its session (`KillUserProcesses` scoped to
#   the greeter user through `KillOnlyUsers`, which a user's own
#   `KillUserProcesses` overrides and a user's own `KillOnlyUsers`
#   replaces outright), ties the settings' identity into logind's unit
#   (so a switch reloads, never restarts, the running logind), leaves
#   the backdrop alone by default and renders a set one as ReGreet's
#   `background.path`; off changes nothing (no logind change either);
#   GDM/SDDM, an explicitly
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
#   on, each fail eval; the shared keymap (`desktop-keys` child) runs
#   with the profile -- the twelve hardware and lock binds, each a
#   `mkDefault` a user `[binds]` entry wins over and each removable
#   through `binds.<name>.enable`, the nine slot-gated binds only
#   while their slot is on, the volume, brightness and mic-mute binds
#   through the audio slot's scripts while that slot is on;
#   the notification daemon (`desktop-notifications` child) runs with
#   the profile -- mako owning `org.freedesktop.Notifications` as a
#   `Type=dbus` user unit (activatable, retried like the bar's unit),
#   its config on the `overlay` layer (popups above fullscreen) and
#   themed by the look unless `theme.targets.notifications.enable`
#   opts out, plus the bar feed (DND state and the unread count into
#   the bar's `push` module, a click toggling DND through `makoctl`,
#   which needs the `push` feature built in); a null package, and a
#   `features` list without `push` beside the feed, each fail eval;
#   the clipboard slot (`desktop-clipboard` child) runs with the
#   profile -- a lean cliphist watched by `wl-paste` (one watcher per
#   selection, into one history), `wl-copy`/`wl-paste` on PATH, the
#   history picker on the keymap's `Super+v` (cliphist through
#   fuzzel's dmenu mode, themed by the look unless
#   `theme.targets.clipboard.enable` opts out) and the history wiped
#   on the idle policy's lock lines; a null tool, a zero `maxItems`
#   and a non-absolute or shell-special `dbPath` each fail eval;
#   the launcher (`desktop-launcher` child) runs with the profile --
#   fuzzel on the keymap's `Super+d` (drun: XDG apps) and
#   `Ctrl+Alt+Space` (run: PATH executables beside apps) through one
#   wrapper script, on the `overlay` layer (above fullscreen) with
#   exclusive keyboard, themed by the look unless
#   `theme.targets.launcher.enable` opts out, holding nothing when
#   closed; a null package and an unknown `daemon` each fail eval;
#   the capture stack (`desktop-capture` child) runs with the
#   profile -- the portal backends (`xdg-desktop-portal-wlr` 0.8.4
#   or later for ScreenCast/Screenshot, `-gtk` for the rest, the
#   `scoot` backend selection on both the system and the per-user
#   config), PipeWire running for the cast, `grim` 1.5.0 or later
#   plus `slurp` for the keymap's three screenshot binds (`Print`
#   every output to file, `Shift+Print` a region to file,
#   `Ctrl+Print` a region to the clipboard), and the output chooser
#   xdpw asks before each cast (a dmenu list through fuzzel by
#   default, click-to-pick through slurp or no picker on a fixed
#   output, themed by the look unless
#   `theme.targets.capture.enable` opts out); a null tool, a
#   too-old backend or grim, a negative frame cap and an empty
#   fixed output each fail eval;
#   the night light (`desktop-nightlight` child) runs with the
#   profile -- wlsunset on the manual schedule (day 6500, the look's
#   own night warmth, 07:00/19:00 over 15 min, no location or network
#   needed) as a user unit, gammastep for location-based
#   sunrise/sunset, themed by the look unless
#   `theme.targets.nightlight.enable` opts out; a null tool, a
#   temperature outside 1000..10000, a night bluer than the day, a
#   sunrise/sunset outside 24-hour `HH:MM`, a transition outside
#   0..7200, a multiplier outside 0.1..10, a coordinate outside its
#   degrees, one coordinate without the other, and gammastep with no
#   location each fail eval;
#   the power policy (`desktop-power` child) is opt-in (never with
#   the profile): PPD as the system service plus the keymap's
#   `Super+p` switch (power-saver, balanced, performance, through
#   `scoot-power-profile` or `powerprofilesctl`, with opt-in
#   auto-switch on AC transitions), the lid and power-key actions on
#   the canonical logind path (docked locks, never suspends;
#   logout never kills user processes), low-battery suspend through
#   UPower (Suspend at 2%, not the HybridSleep default that fails
#   without persistent swap), and the desk-aware charge-limit service
#   (80% default, full-once, trip; inert without the sysfs node); a
#   null daemon, a cap outside 1..100, a blank battery name and a
#   low-battery percent above UPower's critical each fail eval;
#   the audio slot (`desktop-audio-osd` child) runs with the
#   profile -- PipeWire with WirePlumber running for the keymap's
#   volume binds, the volume, brightness and mic-mute binds routed
#   through scripts that also poke the OSD (wob on the `overlay`
#   layer, above fullscreen, themed by the look unless
#   `theme.targets.osd.enable` opts out), and a sink helper
#   (`list`, `set`, `cycle`) for the future picker; a null OSD or
#   dump tool, a negative hide timeout and an unknown `daemon` each
#   fail eval;
#   every profile unit (the idle pair, mako, the bar feed, both
#   every profile unit (the idle pair, mako, the bar feed, both
#   clipboard watchers, the night light, the OSD, and the
#   profile-managed bar -- never the standalone bar) starts in `scoot-session.target`,
#   never the shared `graphical-session.target`, so no other desktop
#   starts them; the home-manager side installs that target itself
#   (present exactly while a unit can want it), which is what carries
#   a launcher-less setup too;
#   every remaining future slot
#   defaults off and inert; `enable` without scoot, a look without the
#   profile, and an unknown look each fail eval;
#   `desktop.greeter` is the greeter (the session entry forced on beside
#   it); and `bar.enable = false` leaves the bar entirely alone. The
#   profile and the policy are Linux-only: off Linux each tool defaults
#   to null, `lock.command` to its bare form, and the policy's own
#   assertions refuse loudly -- pinned, not skipped.
# - the documented install configs (site/src/content/docs/desktop/index.md,
#   "Set up the flake" / NixOS / Home Manager): the minimal NixOS desktop,
#   the GPU-package plus opt-in-greeter variant, and standalone
#   home-manager evaluate through the flake wrappers with the profile on
#   and the moonrise look rendered into the config file -- so the page's
#   snippets cannot rot (nix flake check runs this; keep the page in step).
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
    # what its unit is pinned against. The profile's own session scope
    # (`scoot-session.target`, which the home-manager side installs
    # itself) rides alongside: this is what the target's presence and
    # every unit's binding are pinned against.
    options.systemd.user.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    options.systemd.user.targets = lib.mkOption {
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
    # default, "ignore", then applies). Strings, bools and string
    # lists: nixpkgs declares `KillUserProcesses` a bool (defaulting
    # to false, which the power policy leaves alone -- the real-NixOS
    # pin below checks that inherited default), and the greeter sets
    # `KillOnlyUsers = [ "greeter" ]` through the freeform leaves.
    options.services.logind.settings.Login = lib.mkOption {
      type = lib.types.attrsOf (
        lib.types.either lib.types.str (lib.types.either lib.types.bool (lib.types.listOf lib.types.str))
      );
      default = { };
    };
    # The locker's PAM service (any key goes: the real option is an
    # attrs-of-submodule, this proves presence, not its schema).
    options.security.pam.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    # The capture slot's portal service and PipeWire: the real options
    # are nixpkgs' own (`services/xdg/portal.nix` for `xdg.portal`,
    # `services/desktops/pipewire` for `services.pipewire`); these
    # prove the values land, not their schemas.
    options.xdg.portal.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
    };
    options.xdg.portal.extraPortals = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = [ ];
    };
    options.xdg.portal.config = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    options.services.pipewire.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
    };
    # The power policy's system services (PPD, UPower, udev rules,
    # system units, tmpfiles): plain values the pins read, like the
    # logind and PAM stubs above. The real options merge repeated
    # `extraRules` the same way (nixpkgs' `lines` type concatenates).
    options.services.power-profiles-daemon = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    options.services.upower = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    options.services.udev.extraRules = lib.mkOption {
      type = lib.types.lines;
      default = "";
    };
    options.systemd.services = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    options.systemd.timers = lib.mkOption {
      type = lib.types.attrsOf lib.types.raw;
      default = { };
    };
    options.systemd.tmpfiles.rules = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
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

  # The notification daemon's package as the modules default it: the
  # lean mako (no GTK stack -- `nix/modules/notifications-mako.nix`).
  # The same expression the two `package` defaults import, so the pins
  # below test what ships. Null off Linux, where `pkgs.mako` refuses
  # evaluation when forced.
  leanMako = if isLinux then import ./modules/notifications-mako.nix { inherit pkgs; } else null;
  # The lean daemon's runtime closure, as store paths (for the
  # no-GTK-stack content check). Building it needs no network; off
  # Linux it is never referenced.
  leanMakoClosure = if isLinux then pkgs.closureInfo { rootPaths = [ leanMako ]; } else null;
  # The clipboard manager's package as the modules default it: the lean
  # cliphist (no contrib pickers -- `nix/modules/clipboard-cliphist.nix`).
  # The same expression the two `managerPackage` defaults import, so the
  # pins below test what ships. Null off Linux, where `pkgs.cliphist`
  # refuses evaluation when forced.
  leanClip = if isLinux then import ./modules/clipboard-cliphist.nix { inherit pkgs; } else null;
  # The lean manager's runtime closure, as store paths (for the
  # no-picker-fat content check). Building it needs no network; off
  # Linux it is never referenced.
  leanClipClosure = if isLinux then pkgs.closureInfo { rootPaths = [ leanClip ]; } else null;

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
  # The power policy in a real NixOS evaluation: the daemon, UPower,
  # logind and the charge units against nixpkgs' own modules (not the
  # stubs) -- in particular PPD's real service, UPower's real
  # percentage options, and the udev rule merge.
  osRealPower = evalRealNixos {
    enable = true;
    desktop.enable = true;
    desktop.power.enable = true;
  };
  # The greeter beside the power policy in a real NixOS evaluation:
  # both halves' logind keys present together and rendered (N3 -- the
  # two switches compose, they do not shadow each other).
  osRealGreeterPower = evalRealNixos {
    enable = true;
    desktop.enable = true;
    desktop.power.enable = true;
    greeter.enable = true;
  };
  hmFlake = evalHomeWith flake.homeModule pkgs {
    enable = true;
    settings.wallpaper.color = "#1e1e2e";
  };
  # The same home configuration on a Mac: evaluates, installs the
  # client-only scoot, renders the table as written for the Linux box it
  # deploys to.
  hmDarwin = evalHomeWith flake.homeModule darwinPkgs {
    enable = true;
    settings.wallpaper.image = "~/Pictures/hills.jpg";
  };
  # --- the documented install configs (site/src/content/docs/desktop/index.md:
  # "Set up the flake" / NixOS / Home Manager — keep in step with that
  # page). The minimal NixOS desktop, exactly the page's snippet, through
  # the flake wrapper a consumer imports.
  osDocsDesktop = evalNixosWith [ flake.nixosModule ] pkgs {
    enable = true;
    desktop.enable = true;
    desktop.look = "moonrise";
  };
  # ...plus the page's two commented options switched on: the GPU
  # package choice and the opt-in greeter (with the login user a real
  # configuration provides, like the sibling greeter pins).
  osDocsDesktopFull = evalNixosWith [ flake.nixosModule greeterUser ] pkgs {
    enable = true;
    desktop.enable = true;
    desktop.look = "moonrise";
    package = built.scoot-gpu;
    greeter.enable = true;
  };
  # Standalone home-manager desktop: the page's home.nix snippet, through
  # the legacy `homeManagerModules` spelling (the page documents both;
  # the current `homeModules` spelling is pinned equal below). (As a
  # NixOS module it is the same module under `home-manager.users.<name>`
  # — see the page — so this evaluation covers both forms.)
  hmDocsDesktop = evalHomeWith flake.homeManagerModules.scoot pkgs {
    enable = true;
    desktop.enable = true;
    desktop.look = "moonrise";
  };
  hmDocsDesktopCurrent = evalHomeWith flake.homeModules.scoot pkgs {
    enable = true;
    desktop.enable = true;
    desktop.look = "moonrise";
  };
  # The same documented options through the pure module (stub packages,
  # like the sibling look pins): what the rendered-file check below reads.
  # The flake wrapper above only injects package defaults, pinned by
  # `hmFlake`; the options and the look render identically here.
  hmDocsDesktopPure = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "moonrise";
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
  # A keymap slot script's `bin/` path, found in an evaluation's
  # installed packages by derivation name (the store hash is not
  # knowable in the pin, the name is).
  slotScriptBin =
    eval: name:
    let
      found = lib.findFirst (
        p: (p.name or "") == name
      ) (throw "no ${name} in home.packages") eval.config.home.packages;
    in
    "${found}/bin/${name}";
  # The script derivation itself, for package-list pins (the picker is
  # installed beside the binds, so every profile-packages list with
  # the slot on names it).
  slotScriptDrv =
    eval: name:
    lib.findFirst (
      p: (p.name or "") == name
    ) (throw "no ${name} in home.packages") eval.config.home.packages;

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
  # ...with the user's own `KillUserProcesses`: kept verbatim (a plain
  # assignment beats the greeter's `mkDefault`, so a user who manages
  # logind themselves still evaluates with their value standing).
  osGreeterLogindOverride = evalNixosRegreet {
    enable = true;
    package = fakePkg;
    greeter.enable = true;
  } (greeterUser // { services.logind.settings.Login.KillUserProcesses = false; });
  # ...with the user's own `KillOnlyUsers`: it replaces the greeter's
  # list, it does not merge with it (plain beats `mkDefault` before any
  # concatenation runs), so a list without `"greeter"` opts the greeter
  # back out of the cleanup.
  osGreeterKillOnlyUsersOverride = evalNixosRegreet {
    enable = true;
    package = fakePkg;
    greeter.enable = true;
  } (greeterUser // { services.logind.settings.Login.KillOnlyUsers = [ "alice" ]; });

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
  # nothing installed beyond the profile's own -- auth, whose child
  # wires nothing yet).
  hmDeskSlotOn = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.auth.enable = true;
  };

  # --- shared keymap (`programs.scoot.desktop.keys`) evaluations ---
  #
  # The profile alone: the keymap on, future slots off -- the twelve
  # keymap-owned binds (brightness, volume, mute, media, lock) render,
  # the slot-gated seven stay out.
  hmKeys = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
  };
  # ...every future slot on: all twenty-one binds render, the slot
  # scripts beside them.
  hmKeysSlots = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.launcher.enable = true;
    desktop.clipboard.enable = true;
    desktop.notifications.enable = true;
    desktop.capture.enable = true;
  };
  # ...one bind removed: its combo stays unbound, the rest render.
  hmKeysOmit = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.keys.binds.volumeUp.enable = false;
    desktop.keys.binds.lock.enable = false;
  };
  # ...one bind overridden: the user's own `[binds]` entry wins.
  hmKeysOverride = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    settings.binds."XF86AudioRaiseVolume" = "spawn sh -c true";
  };
  # ...a slot bind overridden: the user's entry beats the slot's.
  hmKeysSlotOverride = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.launcher.enable = true;
    settings.binds."super+d" = "spawn foot";
  };
  # ...the whole keymap off: no binds, no tools beyond the profile's
  # own (the idle policy's five and mako stay -- those are the other
  # children).
  hmKeysOff = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.keys.enable = false;
  };
  # ...keys only (policy and locker off): an empty lock action still
  # fails -- it renders into the keymap's `super+escape` bind.
  hmKeysLockCmdEmpty = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.idle.enable = false;
    desktop.idle.lock.enable = false;
    desktop.idle.mediaInhibit.enable = false;
    desktop.idle.lock.command = "";
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

  # --- notification daemon (`programs.scoot.desktop.notifications`) evaluations ---
  #
  # The profile with a look: the whole slot on (mako unit, feed unit,
  # mako config), themed by the look.
  hmNotif = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
  };
  # ...without a look: the daemon runs unthemed (mako's own colors,
  # the DND behavior still on -- pinned by content below).
  hmNotifNoLook = evalHome {
    enable = true;
    desktop.enable = true;
  };
  # ...the daemon off (the profile turns it on, like the idle policy;
  # each switch back off disables just its half).
  hmNotifOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.notifications.enable = false;
  };
  # ...standalone (no profile): the daemon runs, unthemed.
  hmNotifStandalone = evalHome {
    enable = true;
    desktop.notifications.enable = true;
  };
  # ...with settings (a global winning per key -- including the layer
  # default -- plus a new key, verbatim).
  hmNotifSettings = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.notifications.settings = {
      anchor = "bottom-right";
      layer = "top";
      background-color = "#123456";
    };
  };
  # ...opted out of daemon theming (the look leaves mako alone, the
  # settings still apply).
  hmNotifTargetOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.theme.targets.notifications.enable = false;
    desktop.notifications.settings = {
      border-color = "#123456";
    };
  };
  # Refusals: the daemon with no mako to run it (pinned by message in
  # `_notifPins`)...
  hmNotifNoPkg = evalHome {
    enable = true;
    desktop.notifications.enable = true;
    desktop.notifications.package = null;
  };
  # ...and an unknown daemon, which is an option type error (the
  # `enum`'s own message names the valid value), caught here by
  # `tryEval`.
  hmNotifDaemonBogus =
    builtins.tryEval
      (evalHome {
        enable = true;
        desktop.notifications.enable = true;
        desktop.notifications.daemon = "bogus-daemon";
      }).config.programs.scoot.desktop.notifications.daemon;

  # A fake notification toolchain for the feed's behavior tests: stub
  # `makoctl`, `busctl` and `scootbar`, scripted at RUN time through
  # files under `$SCOOT_FEED_TEST_DIR`, so one HM evaluation covers
  # every scenario. `makoctl mode` prints `mode` (exit `mode-code`),
  # `makoctl list` prints `list` (exit `list-code`); `busctl` prints
  # `bus` and exits 0 (the monitor ending); `scootbar` appends its
  # argv to `calls`, prints `bar-err` to stderr and exits `bar-code`.
  # The bridge under test resolves all three by bare name through its
  # wrapper PATH, where this package's `bin` sorts first (no bar
  # module is imported beside it, so `scootbar` stays a bare name
  # too); `jq` stays the real one from the bridge's own inputs.
  feedStubs = pkgs.runCommand "feed-stubs" { } ''
    mkdir -p $out/bin
    cat > $out/bin/makoctl <<'EOF'
    #!${pkgs.runtimeShell}
    case "$1" in
      mode) cat "$SCOOT_FEED_TEST_DIR/mode"; exit "$(cat "$SCOOT_FEED_TEST_DIR/mode-code")" ;;
      list) cat "$SCOOT_FEED_TEST_DIR/list"; exit "$(cat "$SCOOT_FEED_TEST_DIR/list-code")" ;;
      *) echo "unexpected makoctl args: $*" >&2; exit 99 ;;
    esac
    EOF
    cat > $out/bin/busctl <<'EOF'
    #!${pkgs.runtimeShell}
    cat "$SCOOT_FEED_TEST_DIR/bus"
    exit 0
    EOF
    cat > $out/bin/scootbar <<'EOF'
    #!${pkgs.runtimeShell}
    printf '%s\n' "$*" >> "$SCOOT_FEED_TEST_DIR/calls"
    cat "$SCOOT_FEED_TEST_DIR/bar-err" >&2
    exit "$(cat "$SCOOT_FEED_TEST_DIR/bar-code")"
    EOF
    chmod +x $out/bin/makoctl $out/bin/busctl $out/bin/scootbar
  '';
  # The daemon on, running the stubs: the bridge below is the real
  # `scoot-notify-sync` from the module, with its wrapper PATH aimed
  # at the stubs.
  hmNotifFeedTest = evalHome {
    enable = true;
    desktop.notifications.enable = true;
    desktop.notifications.package = feedStubs;
  };
  # The real bridge script under test (the feed unit runs it with
  # `--watch`; the behavior tests run it bare for one sync).
  feedBridge = lib.removeSuffix " --watch" hmNotifFeedTest.config.systemd.user.services.scoot-notify-sync.Service.ExecStart;

  # ...with overridden state icons (all three custom, DND empty): the
  # icon behavior tests run this bridge beside the default one.
  hmNotifFeedIconsTest = evalHome {
    enable = true;
    desktop.notifications.enable = true;
    desktop.notifications.package = feedStubs;
    desktop.notifications.bar.icons = {
      idle = "✉";
      unread = "!";
      dnd = "";
    };
  };
  feedIconsBridge = lib.removeSuffix " --watch" hmNotifFeedIconsTest.config.systemd.user.services.scoot-notify-sync.Service.ExecStart;
  # The check derivation's interpreter: python3 plus fonttools (the
  # state-icon coverage check reads the font's cmap).
  checkPython = python3.withPackages (ps: [ ps.fonttools ]);

  # --- clipboard (`programs.scoot.desktop.clipboard`) evaluations ---
  #
  # The profile with a look: the whole slot on (two watcher units, the
  # tools on PATH), the picker themed by the look.
  hmClip = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
  };
  # ...without a look: the slot runs unthemed (fuzzel's own colors --
  # pinned by content below).
  hmClipNoLook = evalHome {
    enable = true;
    desktop.enable = true;
  };
  # ...the slot off (the profile turns it on, like the idle policy;
  # each switch back off disables just its half).
  hmClipOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.clipboard.enable = false;
  };
  # ...both the clipboard and the capture slots off: `wl-clipboard`
  # leaves entirely (each slot installs the same derivation, so one
  # slot alone keeps it).
  hmClipCaptureOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.clipboard.enable = false;
    desktop.capture.enable = false;
  };
  # ...standalone (no profile): the slot runs, unthemed.
  hmClipStandalone = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
  };
  # ...with history bounds (a longer tail in a moved db -- an
  # absolute path without shell specials, the only shape that passes
  # eval: the store, the picker and the lock wipe must all name the
  # same file).
  hmClipBounds = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.clipboard.maxItems = 250;
    desktop.clipboard.dbPath = "/home/scoot-test/.cache/cliphist-test/db";
  };
  # ...opted out of picker theming (the look leaves fuzzel alone).
  hmClipTargetOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.theme.targets.clipboard.enable = false;
  };
  # ...in radial-burst (whose bar palette has no `hover`: the match
  # highlight falls back to the accent -- pinned by content below).
  hmClipLookBurst = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "radial-burst";
  };
  # Refusals: the slot with each tool missing (pinned by message in
  # `_clipPins`)...
  hmClipNoManager = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.managerPackage = null;
  };
  hmClipNoTools = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.wlClipboardPackage = null;
  };
  hmClipNoMenu = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.menuPackage = null;
  };
  # ...a history of nothing, and a db path in each refused shape (a
  # `~` path would expand on the lock line but stay literal in the
  # scripts, a relative path resolves against three different CWDs, and
  # a space, quote, `$`, backtick or `;` splits or breaks out of the
  # lock line's quoting -- each pinned by message in `_clipPins`).
  hmClipBadMax = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.maxItems = 0;
  };
  hmClipBadDbTilde = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = "~/.cache/cliphist/db";
  };
  hmClipBadDbRelative = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = ".cache/cliphist/db";
  };
  hmClipBadDbSpace = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = "/home/scoot-test/my clips/db";
  };
  hmClipBadDbQuote = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = "/home/scoot-test/cliphist'; reboot #";
  };
  hmClipBadDbDollar = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = "/home/scoot-test/$USER/db";
  };
  hmClipBadDbBacktick = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = "/home/scoot-test/`id`/db";
  };
  hmClipBadDbSemi = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = "/home/scoot-test/cliphist;wipe/db";
  };
  hmClipBadDbGlob = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = "/home/scoot-test/*/db";
  };
  hmClipBadDbPipe = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = "/home/scoot-test/a|b/db";
  };
  hmClipBadDbEmpty = evalHome {
    enable = true;
    desktop.clipboard.enable = true;
    desktop.clipboard.dbPath = "";
  };

  # A fake clipboard toolchain for the store-entry and picker behavior
  # tests: stub `cliphist`, `wl-copy`, `fuzzel` and `scoot`, scripted at
  # RUN time through files under `$SCOOT_CLIP_TEST_DIR`, so one HM
  # evaluation per script covers every scenario. `cliphist` appends its
  # argv plus stdin plus `CLIPBOARD_STATE` to `calls`; `list` prints
  # `list`, `decode` prints `decode-bytes` (exit `decode-code`);
  # `fuzzel` prints `pick` (exit `pick-code`); `wl-copy` copies stdin to
  # `pasted`; `scoot msg locked` prints `{"type":"locked",...}` (exit
  # `locked-code`, flag `locked-value`). The scripts under test resolve
  # every tool by absolute path into this package (the evaluations
  # below point each package option at the stubs, and the lock probe at
  # the stub `scoot` through `package`).
  clipStubs = pkgs.runCommand "clip-stubs" { } ''
    mkdir -p $out/bin
    cat > $out/bin/cliphist <<'EOF'
    #!${pkgs.runtimeShell}
    while :; do case "$1" in -db-path|-max-items) shift 2 ;; *) break ;; esac; done
    case "$1" in
      list) cat "$SCOOT_CLIP_TEST_DIR/list"; exit 0 ;;
      decode)
        printf 'decode-argv:%s\n' "$*" >> "$SCOOT_CLIP_TEST_DIR/calls"
        cat "$SCOOT_CLIP_TEST_DIR/decode-bytes"; exit "$(cat "$SCOOT_CLIP_TEST_DIR/decode-code")" ;;
      store)
        printf 'store-argv:%s state:%s\n' "$*" "$CLIPBOARD_STATE" >> "$SCOOT_CLIP_TEST_DIR/calls"
        cat >> "$SCOOT_CLIP_TEST_DIR/calls"; exit 0 ;;
      wipe) printf 'wipe\n' >> "$SCOOT_CLIP_TEST_DIR/calls"; exit 0 ;;
      *) echo "unexpected cliphist args: $*" >&2; exit 99 ;;
    esac
    EOF
    cat > $out/bin/fuzzel <<'EOF'
    #!${pkgs.runtimeShell}
    cat > "$SCOOT_CLIP_TEST_DIR/menu-input"
    cat "$SCOOT_CLIP_TEST_DIR/pick"; exit "$(cat "$SCOOT_CLIP_TEST_DIR/pick-code")"
    EOF
    cat > $out/bin/wl-copy <<'EOF'
    #!${pkgs.runtimeShell}
    cat > "$SCOOT_CLIP_TEST_DIR/pasted"
    exit 0
    EOF
    cat > $out/bin/wl-paste <<'EOF'
    #!${pkgs.runtimeShell}
    echo "unexpected wl-paste call: $*" >&2; exit 99
    EOF
    cat > $out/bin/scoot <<'EOF'
    #!${pkgs.runtimeShell}
    case "$*" in
      "msg locked") printf '{"type":"locked","locked":%s}\n' "$(cat "$SCOOT_CLIP_TEST_DIR/locked-value")"; exit "$(cat "$SCOOT_CLIP_TEST_DIR/locked-code")" ;;
      *) echo "unexpected scoot args: $*" >&2; exit 99 ;;
    esac
    EOF
    chmod +x $out/bin/cliphist $out/bin/fuzzel $out/bin/wl-copy $out/bin/wl-paste $out/bin/scoot
  '';
  # The slot on, running the stubs: the store entry below is the real
  # `scoot-clipboard-store-entry` from the module (extracted from the
  # watcher unit's `ExecStart`, whose last word is the script), and the
  # picker below is the real `scoot-clipboard-pick` from the keymap
  # (installed beside the binds, found by derivation name like the
  # capture scripts).
  hmClipStoreTest = evalHome {
    enable = true;
    package = clipStubs;
    desktop.clipboard.enable = true;
    desktop.clipboard.managerPackage = clipStubs;
    desktop.clipboard.wlClipboardPackage = clipStubs;
    desktop.clipboard.menuPackage = clipStubs;
  };
  hmClipPickTest = evalHome {
    enable = true;
    package = clipStubs;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.clipboard.enable = true;
    desktop.clipboard.managerPackage = clipStubs;
    desktop.clipboard.wlClipboardPackage = clipStubs;
    desktop.clipboard.menuPackage = clipStubs;
    desktop.launcher.enable = true;
    desktop.notifications.enable = true;
    desktop.capture.enable = true;
  };
  clipEntryOf =
    eval: service:
    lib.last (lib.splitString " " eval.config.systemd.user.services.${service}.Service.ExecStart);
  clipStoreEntry = clipEntryOf hmClipStoreTest "scoot-clipboard-store";
  clipPrimaryEntry = clipEntryOf hmClipStoreTest "scoot-clipboard-primary-store";
  clipPicker = slotScriptBin hmClipPickTest "scoot-clipboard-pick";

  # --- clipboard system evaluations ---
  osClip = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
  };
  osClipOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.clipboard.enable = false;
  };
  osClipNoManager = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.clipboard.managerPackage = null;
  };
  osClipNoTools = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.clipboard.wlClipboardPackage = null;
  };
  osClipNoMenu = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.clipboard.menuPackage = null;
  };

  # --- launcher (`programs.scoot.desktop.launcher`) evaluations ---
  #
  # The profile with a look: the whole slot on (the package on PATH,
  # the wrapper script beside the keymap's binds), the launcher themed
  # by the look.
  hmLaunch = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
  };
  # ...without a look: the slot runs unthemed (fuzzel's own colors --
  # pinned by content below).
  hmLaunchNoLook = evalHome {
    enable = true;
    desktop.enable = true;
  };
  # ...the slot off (the profile turns it on, like the idle policy;
  # the switch back off disables just its half).
  hmLaunchOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.launcher.enable = false;
  };
  # ...standalone (no profile): the binary on PATH, unthemed, no binds
  # (those need the keymap, which the profile turns on).
  hmLaunchStandalone = evalHome {
    enable = true;
    desktop.launcher.enable = true;
  };
  # ...opted out of launcher theming (the look leaves fuzzel alone).
  hmLaunchTargetOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.theme.targets.launcher.enable = false;
  };
  # ...one per remaining look: the themed flags pinned by content
  # below (music-desk rides on `hmLaunch`).
  hmLaunchLookVinyl = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "vinyl-sunset";
  };
  hmLaunchLookBurst = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "radial-burst";
  };
  hmLaunchLookMoon = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "moonrise";
  };
  # Refusals: the slot with no launcher to run it (pinned by message
  # in `_launchPins`)...
  hmLaunchNoPkg = evalHome {
    enable = true;
    desktop.launcher.enable = true;
    desktop.launcher.package = null;
  };
  # ...and an unknown daemon, which is an option type error (the
  # `enum`'s own message names the valid value), caught here by
  # `tryEval`.
  hmLaunchDaemonBogus =
    builtins.tryEval
      (evalHome {
        enable = true;
        desktop.launcher.enable = true;
        desktop.launcher.daemon = "bogus-daemon";
      }).config.programs.scoot.desktop.launcher.daemon;

  # --- launcher system evaluations ---
  osLaunch = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
  };
  osLaunchOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.launcher.enable = false;
  };
  osLaunchNoPkg = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.launcher.package = null;
  };

  # --- capture (`programs.scoot.desktop.capture`) evaluations ---
  #
  # The profile with a look: the whole slot on (the tools on PATH,
  # the per-desktop chooser file), the chooser themed by the look.
  hmCapture = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
  };
  # ...without a look: the slot runs unthemed (fuzzel's and slurp's
  # own colors -- pinned by content below).
  hmCaptureNoLook = evalHome {
    enable = true;
    desktop.enable = true;
  };
  # ...the slot off (the profile turns it on; the switch back off
  # disables just its half: no tools, no chooser file, no binds).
  hmCaptureOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.capture.enable = false;
  };
  # ...standalone (no profile): the tools on PATH and the chooser
  # file written, unthemed, no binds (those need the keymap, which
  # the profile turns on).
  hmCaptureStandalone = evalHome {
    enable = true;
    desktop.capture.enable = true;
  };
  # ...the slurp picker instead of the menu...
  hmCaptureSlurp = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.capture.chooser = "slurp";
  };
  # ...no picker at all, on a named output...
  hmCaptureNoneNamed = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.capture.chooser = "none";
    desktop.capture.outputName = "DP-1";
  };
  # ...no picker, any output...
  hmCaptureNoneAny = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.capture.chooser = "none";
  };
  # ...opted out of chooser theming (the look leaves the menus
  # alone).
  hmCaptureTargetOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.theme.targets.capture.enable = false;
  };
  # Refusals: a null beside `enable` (each pinned by message in
  # `_capturePins`)...
  hmCaptureNoGrim = evalHome {
    enable = true;
    desktop.capture.enable = true;
    desktop.capture.grimPackage = null;
  };
  hmCaptureNoSlurp = evalHome {
    enable = true;
    desktop.capture.enable = true;
    desktop.capture.slurpPackage = null;
  };
  hmCaptureNoMenu = evalHome {
    enable = true;
    desktop.capture.enable = true;
    desktop.capture.menuPackage = null;
  };
  hmCaptureNoCopy = evalHome {
    enable = true;
    desktop.capture.enable = true;
    desktop.capture.wlClipboardPackage = null;
  };
  # ...a too-old grim (1.4.0 speaks only the wlr screencopy protocol
  # scoot omits on purpose)...
  hmCaptureOldGrim = evalHome {
    enable = true;
    desktop.capture.enable = true;
    desktop.capture.grimPackage = pkgs.grim.overrideAttrs (old: {
      version = "1.4.0";
    });
  };
  # ...a negative frame cap, and an empty fixed output...
  hmCaptureNegFps = evalHome {
    enable = true;
    desktop.capture.enable = true;
    desktop.capture.maxFps = -1;
  };
  hmCaptureEmptyOutput = evalHome {
    enable = true;
    desktop.capture.enable = true;
    desktop.capture.chooser = "none";
    desktop.capture.outputName = "";
  };
  # ...and an unknown chooser, which is an option type error (the
  # `enum`'s own message names the valid values), caught here by
  # `tryEval`.
  hmCaptureChooserBogus =
    builtins.tryEval
      (evalHome {
        enable = true;
        desktop.capture.enable = true;
        desktop.capture.chooser = "bogus-chooser";
      }).config.programs.scoot.desktop.capture.chooser;

  # --- capture system evaluations ---
  osCapture = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
  };
  osCaptureOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.capture.enable = false;
  };
  # ...both PipeWire users off: the service stops with them.
  osCaptureAudioOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.capture.enable = false;
    desktop.audio.enable = false;
  };
  osCaptureStandalone = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.capture.enable = true;
  };
  osCaptureNoWlr = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.capture.portalWlrPackage = null;
  };
  osCaptureNoGtk = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.capture.portalGtkPackage = null;
  };
  osCaptureNoGrim = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.capture.grimPackage = null;
  };
  osCaptureNoSlurp = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.capture.slurpPackage = null;
  };
  osCaptureNoMenu = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.capture.menuPackage = null;
  };
  # ...a 0.8.3 backend (stalls recordings) and a too-old grim...
  osCaptureOldWlr = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.capture.portalWlrPackage = pkgs.xdg-desktop-portal-wlr.overrideAttrs (old: {
      version = "0.8.3";
    });
  };
  osCaptureOldGrim = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.capture.grimPackage = pkgs.grim.overrideAttrs (old: {
      version = "1.4.0";
    });
  };
  # --- night light (`programs.scoot.desktop.nightlight`) evaluations ---
  #
  # The profile with a look: the whole slot on (the tool on PATH, the
  # user unit), warming to the look's own night temperature.
  hmNight = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
  };
  # ...without a look: the slot runs on the plain default (3500 K).
  hmNightNoLook = evalHome {
    enable = true;
    desktop.enable = true;
  };
  # ...the slot off (the profile turns it on, like the idle policy;
  # the switch back off disables just its half: no tool, no unit).
  hmNightOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.nightlight.enable = false;
  };
  # ...standalone (no profile): the tool and the unit, unthemed, with
  # the session scope installed for it.
  hmNightStandalone = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
  };
  # ...opted out of look theming (the plain default stands while the
  # rest follows the look, a user value winning either way).
  hmNightTargetOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.theme.targets.nightlight.enable = false;
  };
  # ...a user night temperature beside a look (winning per key over
  # the look's own).
  hmNightUserWins = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.nightlight.nightTemp = 3000;
  };
  # ...location mode (the sun computes the boundaries: the manual pair
  # and the duration go unread)...
  hmNightLocated = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.nightlight.latitude = 37.33;
    desktop.nightlight.longitude = -121.89;
  };
  # ...and the gammastep daemon (location-based: the only mode it has).
  hmNightGamma = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.nightlight.daemon = "gammastep";
    desktop.nightlight.latitude = 37.33;
    desktop.nightlight.longitude = -121.89;
  };
  # Refusals: the slot with no tool to run it (pinned by message in
  # `_nightlightPins`)...
  hmNightNoPkg = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.package = null;
  };
  # ...a day outside 1000..10000, a night outside it, and a night
  # bluer than the day...
  hmNightBadDay = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.dayTemp = 20000;
  };
  hmNightBadNight = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.nightTemp = 500;
  };
  hmNightNightAboveDay = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.dayTemp = 4000;
    desktop.nightlight.nightTemp = 5000;
  };
  # ...a sunrise/sunset outside 24-hour `HH:MM`...
  hmNightBadRise = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.sunrise = "7am";
  };
  hmNightBadSet = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.sunset = "25:00";
  };
  # ...a transition outside 0..7200, and a multiplier outside
  # 0.1..10...
  hmNightBadDuration = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.duration = 10800;
  };
  hmNightBadGamma = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.gamma = 0.05;
  };
  # ...a latitude/longitude outside its degrees, and one coordinate
  # without the other...
  hmNightBadLat = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.latitude = 95;
    desktop.nightlight.longitude = -121.89;
  };
  hmNightHalfLoc = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.latitude = 37.33;
  };
  # ...gammastep with nowhere to stand (geoclue is not wired)...
  hmNightGammaNoLoc = evalHome {
    enable = true;
    desktop.nightlight.enable = true;
    desktop.nightlight.daemon = "gammastep";
  };
  # ...and an unknown daemon, which is an option type error (the
  # `enum`'s own message names the valid values), caught here by
  # `tryEval`.
  hmNightDaemonBogus =
    builtins.tryEval
      (evalHome {
        enable = true;
        desktop.nightlight.enable = true;
        desktop.nightlight.daemon = "bogus-daemon";
      }).config.programs.scoot.desktop.nightlight.daemon;

  # --- night light system evaluations ---
  osNight = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
  };
  osNightOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.nightlight.enable = false;
  };
  osNightStandalone = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.nightlight.enable = true;
  };
  osNightNoPkg = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.nightlight.package = null;
  };
  # --- power policy (`programs.scoot.desktop.power`) evaluations ---
  #
  # Opt-in (never with the profile): the daemon, the logind/UPower
  # policy and the charge service, plus the keymap's profile bind.
  hmPower = evalHome {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.power.enable = true;
  };
  # ...standalone (no profile): the switch and the CLI run unthemed.
  hmPowerStandalone = evalHome {
    enable = true;
    desktop.power.enable = true;
  };
  # ...the charge cap off: the daemon and the lid policy without it.
  hmPowerChargeOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.chargeLimit.enable = false;
  };
  # ...with charge bounds (a lower cap on a named battery, an hourly
  # trip, a half-day trip end).
  hmPowerChargeBounds = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.chargeLimit.limit = 70;
    desktop.power.chargeLimit.battery = "BAT0";
    desktop.power.chargeLimit.fullAfter = 3600;
    desktop.power.chargeLimit.tripEndsAfter = 43200;
  };
  # ...with auto-switch and lid overrides (values are plain: the udev
  # merge they render is pinned on the NixOS side below).
  hmPowerAuto = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.profileOnAC = "performance";
    desktop.power.profileOnBattery = "power-saver";
  };
  hmPowerLid = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.lidSwitch = "lock";
    desktop.power.powerKey = "ignore";
  };
  # ...with a PowerOff trip (no risky-action flag for it).
  hmPowerLowAction = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.lowBattery.action = "PowerOff";
  };
  # ...with the profiles daemon dropped (no PPD driver honors it):
  # no switch script, no daemon package, no bind -- while the lid,
  # low-battery and charge-limit policy still applies.
  hmPowerNoProfiles = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.profiles.enable = false;
  };
  # ...and a bad charge bound with the policy off: the refusal still
  # fires (the bounds sit outside `power.enable`, like the home-side
  # ones always did).
  hmChargeBadLimitNoPower = evalHome {
    enable = true;
    desktop.power.chargeLimit.enable = true;
    desktop.power.chargeLimit.limit = 101;
  };
  # Refusals: the policy with no daemon to run it (pinned by message
  # in `_powerPins`)...
  hmPowerNoPkg = evalHome {
    enable = true;
    desktop.power.enable = true;
    desktop.power.profiles.package = null;
  };
  # ...a cap outside 1..100, and a blank battery name (each pinned by
  # message in `_powerPins`)...
  hmPowerBadLimit = evalHome {
    enable = true;
    desktop.power.enable = true;
    desktop.power.chargeLimit.limit = 101;
  };
  hmPowerBadBattery = evalHome {
    enable = true;
    desktop.power.enable = true;
    desktop.power.chargeLimit.battery = "   ";
  };
  # ...and a low-battery percent above UPower's critical (pinned by
  # message in `_powerPins`).
  hmPowerBadLowPct = evalHome {
    enable = true;
    desktop.power.enable = true;
    desktop.power.lowBattery.percentage = 10;
  };

  # --- power policy system evaluations ---
  #
  # The profile with the policy: the daemon installed, the lid and
  # power-key actions on the canonical logind path, low-battery
  # suspend through UPower, the charge service, timer and udev rules
  # present, still additive (no default session).
  osPower = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
  };
  # ...standalone (no profile): the policy without the session entry.
  osPowerStandalone = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.power.enable = true;
  };
  # ...the profile without the policy: no daemon, no UPower, logind
  # untouched, no charge units.
  osPowerOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
  };
  # ...the charge cap off: the daemon and the lid policy without it.
  osPowerChargeOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.chargeLimit.enable = false;
  };
  # ...with auto-switch set: the udev rules select profiles on AC
  # transitions.
  osPowerAuto = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.profileOnAC = "performance";
    desktop.power.profileOnBattery = "power-saver";
  };
  # ...with an explicit lid action: the user's value wins over the
  # policy default (a separate module, the way the greeter's
  # overrides ride along -- the evaluation above can only set
  # `programs.scoot`).
  osPowerLidOverride =
    evalNixosWith
      [
        ./modules/nixos.nix
        { programs.scoot.wallpaper.package = lib.mkDefault fakeBg; }
        { services.logind.settings.Login.HandleLidSwitch = "ignore"; }
      ]
      pkgs
      {
        enable = true;
        package = fakePkg;
        desktop.enable = true;
        desktop.power.enable = true;
      };
  # ...with a PowerOff trip: no risky-action flag beside it.
  osPowerLowAction = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.lowBattery.action = "PowerOff";
  };
  # ...with the profiles daemon dropped (no PPD driver honors it):
  # the daemon stays out while the lid, low-battery and
  # charge-limit policy still applies.
  osPowerNoProfiles = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.profiles.enable = false;
  };
  # ...with the user's own service values beside the policy: plain
  # assignments win over the policy's `mkDefault`s without an eval
  # error (a separate module, the way the greeter's overrides ride
  # along -- the evaluation above can only set `programs.scoot`).
  osPowerServiceOverride =
    evalNixosWith
      [
        ./modules/nixos.nix
        { programs.scoot.wallpaper.package = lib.mkDefault fakeBg; }
        { services.power-profiles-daemon.enable = false; }
        { services.upower.enable = false; }
      ]
      pkgs
      {
        enable = true;
        package = fakePkg;
        desktop.enable = true;
        desktop.power.enable = true;
      };
  # ...with the docked-lid rule set to suspend: the user's value wins
  # (the idle child's twin defers while the policy runs) instead of
  # eval-conflicting with it.
  osPowerDockedSuspend = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.lidSwitchDocked = "suspend";
  };
  # ...and a bad charge bound with the policy off: the refusal still
  # fires (the bounds sit outside `power.enable`).
  osChargeBadLimitNoPower = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.power.chargeLimit.enable = true;
    desktop.power.chargeLimit.limit = 0;
  };
  # Refusals: the policy with no daemon, a cap outside 1..100, a
  # blank battery name, and a low-battery percent above UPower's
  # critical (each pinned by message in `_powerPins`).
  osPowerNoPkg = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.profiles.package = null;
  };
  osPowerBadLimit = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.chargeLimit.limit = 0;
  };
  osPowerBadBattery = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.chargeLimit.battery = "";
  };
  osPowerBadLowPct = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.power.enable = true;
    desktop.power.lowBattery.percentage = 6;
  };

  # A fake PPD for the profile-switch behavior tests: stub
  # `powerprofilesctl`, scripted at RUN time through files under
  # `$SCOOT_POWER_TEST_DIR`, so one HM evaluation covers every
  # scenario. `get` prints the last `set` profile (`current` before
  # any set) and exits `get-code`; `set` appends its profile to
  # `sets` and exits `set-code`; `list` prints the `list` file (the
  # daemon's offered profiles in its own format, which `cycle`
  # skips against). Named `powerprofilesctl`, so the
  # switch script's `lib.getExe` resolves to this binary (the
  # evaluation below points `profiles.package` at the stubs).
  ppdStubs =
    (pkgs.runCommand "powerprofilesctl" { } ''
      mkdir -p $out/bin
      cat > $out/bin/powerprofilesctl <<'EOF'
      #!${pkgs.runtimeShell}
      case "$1" in
        get)
          if [ -s "$SCOOT_POWER_TEST_DIR/sets" ]; then tail -n 1 "$SCOOT_POWER_TEST_DIR/sets"; else cat "$SCOOT_POWER_TEST_DIR/current"; fi
          exit "$(cat "$SCOOT_POWER_TEST_DIR/get-code")" ;;
        set) printf '%s\n' "$2" >> "$SCOOT_POWER_TEST_DIR/sets"; exit "$(cat "$SCOOT_POWER_TEST_DIR/set-code")" ;;
        list) cat "$SCOOT_POWER_TEST_DIR/list" ;;
        *) echo "unexpected powerprofilesctl args: $*" >&2; exit 99 ;;
      esac
      EOF
      chmod +x $out/bin/powerprofilesctl
    '')
    // {
      meta = {
        mainProgram = "powerprofilesctl";
      };
    };
  # The slot on, running the stubs: the switch below is the real
  # `scoot-power-profile` from the keymap (installed beside the
  # binds, found by derivation name like the capture scripts -- the
  # keymap is on explicitly, since there is no profile here to turn
  # it on).
  hmPowerScriptTest = evalHome {
    enable = true;
    desktop.keys.enable = true;
    desktop.power.enable = true;
    desktop.power.profiles.package = ppdStubs;
  };
  powerSwitch = slotScriptBin hmPowerScriptTest "scoot-power-profile";
  # The charge script with no trip (`fullAfter = 0`): the trip
  # scenario below runs this build beside the default one.
  chargeScriptNoTrip = import ./modules/power-charge.nix {
    inherit pkgs lib;
    limit = 80;
    fullAfter = 0;
    tripEndsAfter = 86400;
    battery = null;
  };

  # --- audio (`programs.scoot.desktop.audio`) evaluations ---
  #
  # The profile with a look: the whole slot on (the OSD and its unit,
  # the control scripts, the sink helper), the OSD themed by the look.
  hmAudio = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
  };
  # ...without a look: the slot runs unthemed (wob's own colors --
  # pinned by content below).
  hmAudioNoLook = evalHome {
    enable = true;
    desktop.enable = true;
  };
  # ...the slot off (the profile turns it on; the switch back off
  # disables just its half: no OSD, no scripts, the binds run silent).
  hmAudioOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.audio.enable = false;
  };
  # ...standalone (no profile): the slot runs, unthemed, its unit in
  # the session scope.
  hmAudioStandalone = evalHome {
    enable = true;
    desktop.audio.enable = true;
  };
  # ...retimed (a longer on-screen hold)...
  hmAudioTimeout = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.audio.osd.timeoutMs = 2500;
  };
  # ...opted out of OSD theming (the look leaves wob alone, the muted
  # style still defined so mute never names a missing style).
  hmAudioTargetOff = evalHome {
    enable = true;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.theme.targets.osd.enable = false;
  };
  # Refusals: the slot with no OSD to run (pinned by message in
  # `_audioPins`)...
  hmAudioNoPkg = evalHome {
    enable = true;
    desktop.audio.enable = true;
    desktop.audio.osd.package = null;
  };
  # ...with no dump tool for the sink helper...
  hmAudioNoDump = evalHome {
    enable = true;
    desktop.audio.enable = true;
    desktop.audio.dumpPackage = null;
  };
  # ...with a negative hide timeout...
  hmAudioNegTimeout = evalHome {
    enable = true;
    desktop.audio.enable = true;
    desktop.audio.osd.timeoutMs = -1;
  };
  # ...and an unknown daemon, which is an option type error (the
  # `enum`'s own message names the valid value), caught here by
  # `tryEval`.
  hmAudioDaemonBogus =
    builtins.tryEval
      (evalHome {
        enable = true;
        desktop.audio.enable = true;
        desktop.audio.daemon = "bogus-daemon";
      }).config.programs.scoot.desktop.audio.daemon;

  # --- audio system evaluations ---
  osAudio = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
  };
  osAudioOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.audio.enable = false;
  };
  osAudioStandalone = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.audio.enable = true;
  };
  osAudioTimeout = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.audio.osd.timeoutMs = 3000;
  };
  osAudioNoPkg = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.audio.osd.package = null;
  };
  osAudioNoDump = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.audio.dumpPackage = null;
  };

  # A fake audio toolchain for the control-script behavior tests:
  # stub `wpctl`, `brightnessctl`, `pw-dump` and `wob`, scripted at
  # RUN time through files under `$SCOOT_AUDIO_TEST_DIR`, so one HM
  # evaluation covers every scenario. `wpctl get-volume` prints `vol`
  # (exit `vol-code`); `set-volume`/`set-mute` append their argv to
  # `calls` (exit `set-code`); `inspect` prints `inspect` (exit 0);
  # `set-default` appends to `calls` and records the id in
  # `default-id`; `brightnessctl -m -l` prints `devs` (exit 0);
  # `brightnessctl set` appends to `calls`; `pw-dump` prints `dump`
  # (exit `dump-code`); `wob` is never run (the OSD writer is tested
  # against a fifo, not the daemon). `jq`, `awk`, `tail` and `mkfifo`
  # stay the real ones from the scripts' own references.
  audioStubs = pkgs.runCommand "audio-stubs" { } ''
    mkdir -p $out/bin
    cat > $out/bin/wpctl <<'EOF'
    #!${pkgs.runtimeShell}
    case "$1" in
      get-volume) cat "$SCOOT_AUDIO_TEST_DIR/vol"; exit "$(cat "$SCOOT_AUDIO_TEST_DIR/vol-code")" ;;
      set-volume|set-mute) printf '%s\n' "wpctl-$1 $*" >> "$SCOOT_AUDIO_TEST_DIR/calls"; exit "$(cat "$SCOOT_AUDIO_TEST_DIR/set-code")" ;;
      inspect) cat "$SCOOT_AUDIO_TEST_DIR/inspect"; exit 0 ;;
      set-default) printf '%s\n' "set-default $2" >> "$SCOOT_AUDIO_TEST_DIR/calls"; printf '%s' "$2" > "$SCOOT_AUDIO_TEST_DIR/default-id"; exit 0 ;;
      *) echo "unexpected wpctl args: $*" >&2; exit 99 ;;
    esac
    EOF
    cat > $out/bin/brightnessctl <<'EOF'
    #!${pkgs.runtimeShell}
    case "$*" in
      *"-m -l"*) cat "$SCOOT_AUDIO_TEST_DIR/devs"; exit 0 ;;
      *" set "*) printf '%s\n' "brightness-set $*" >> "$SCOOT_AUDIO_TEST_DIR/calls"; exit "$(cat "$SCOOT_AUDIO_TEST_DIR/set-code")" ;;
      *) echo "unexpected brightnessctl args: $*" >&2; exit 99 ;;
    esac
    EOF
    cat > $out/bin/pw-dump <<'EOF'
    #!${pkgs.runtimeShell}
    cat "$SCOOT_AUDIO_TEST_DIR/dump"; exit "$(cat "$SCOOT_AUDIO_TEST_DIR/dump-code")"
    EOF
    cat > $out/bin/wob <<'EOF'
    #!${pkgs.runtimeShell}
    echo "unexpected wob run: $*" >&2; exit 99
    EOF
    chmod +x $out/bin/wpctl $out/bin/brightnessctl $out/bin/pw-dump $out/bin/wob
  '';
  # The slot on, running the stubs: every package option points at
  # them, so the scripts under test resolve all three controls by
  # absolute path into this package.
  hmAudioBehavior = evalHome {
    enable = true;
    desktop.audio.enable = true;
    desktop.keys.volumePackage = audioStubs;
    desktop.keys.brightnessPackage = audioStubs;
    desktop.audio.dumpPackage = audioStubs;
    desktop.audio.osd.package = audioStubs;
  };
  audioBehaviorVolume = slotScriptBin hmAudioBehavior "scoot-volume";
  audioBehaviorBrightness = slotScriptBin hmAudioBehavior "scoot-brightness";
  audioBehaviorSink = slotScriptBin hmAudioBehavior "scoot-audio-sink";
  audioBehaviorOsd = slotScriptBin hmAudioBehavior "scoot-osd";
  # The retimed OSD config (what the unit runs the daemon with).
  audioTimeoutIni = hmAudioTimeout.config.xdg.configFile."wob/wob.ini".source;

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
    desktop.auth.enable = true;
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
  # The shared keymap off: the profile's other packages only.
  osKeysOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.keys.enable = false;
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
  # --- notification daemon system evaluations ---
  #
  # The profile: mako installed beside scoot and scootbg, still
  # additive (no default session).
  osNotif = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
  };
  # ...the daemon off: the profile's own packages only.
  osNotifOff = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.notifications.enable = false;
  };
  # Refusal: the daemon with no mako to install (pinned by message in
  # `_notifPins`).
  osNotifNoPkg = evalNixos {
    enable = true;
    package = fakePkg;
    desktop.enable = true;
    desktop.notifications.package = null;
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
  # A standalone bar (no profile at all): the unit stays a generic
  # `graphical-session.target` unit, the way other compositors run it.
  hmBarStandalone =
    evalHomeDesktop
      {
        enable = true;
        package = fakePkg;
      }
      {
        enable = true;
        package = fakeBar;
      };
  osBarStandalone =
    evalNixosDesktop
      {
        enable = true;
        package = fakePkg;
      }
      {
        enable = true;
        package = fakeBar;
      };
  # ...and beside a profile that leaves the bar alone (the
  # `hmDeskBarOff`/`osDeskBarOff` shape with the user's own bar enabled
  # instead of none): still the shared target, never the scoot scope.
  hmDeskBarOffStandalone =
    evalHomeDesktop
      {
        enable = true;
        package = fakePkg;
        desktop.enable = true;
        desktop.look = "music-desk";
        desktop.bar.enable = false;
      }
      {
        enable = true;
        package = fakeBar;
      };
  osDeskBarOffStandalone =
    evalNixosDesktop
      {
        enable = true;
        package = fakePkg;
        desktop.enable = true;
        desktop.look = "radial-burst";
        desktop.bar.enable = false;
      }
      {
        enable = true;
        package = fakeBar;
      };
  # The notification feed's bar half: the profile beside the bar
  # module (music-desk, so the daemon runs themed).
  hmDeskBarNotif = evalHomeDesktop {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "music-desk";
  } { package = fakeBar; };
  # ...with the daemon off: no feed module (the bar is entirely the
  # user's, the push table absent).
  hmDeskBarNoNotif = evalHomeDesktop {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.notifications.enable = false;
  } { package = fakeBar; };
  # The power policy beside the bar module: the charge button's fill
  # unit renders (its `ExecStart` names the system-wide `scoot-charge`
  # by absolute path -- pinned in `_powerPins`, which fails on a bare
  # name).
  hmPowerBar = evalHomeDesktop {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.power.enable = true;
  } { package = fakeBar; };
  # ...with overridden state icons (custom idle, empty DND): the bar
  # half follows per key, and the feed bridge carries them.
  hmDeskBarNotifIcons = evalHomeDesktop {
    enable = true;
    package = fakePkg;
    wallpaper.package = fakeBg;
    desktop.enable = true;
    desktop.look = "music-desk";
    desktop.notifications.bar.icons = {
      idle = "✉";
      unread = "!";
      dnd = "";
    };
  } { package = fakeBar; };
  # ...with the daemon on but a bar built without the push module:
  # refused at eval, naming it (pinned by message in `_notifPins`).
  # The stand-in bar is a plain derivation, so it gains the `.override`
  # the features option asks of a real package (returning itself: only
  # the option plumbing is under test here, not a rebuild).
  hmDeskBarNoPush =
    evalHomeDesktopWith [ ]
      {
        enable = true;
        package = fakePkg;
        wallpaper.package = fakeBg;
        desktop.enable = true;
        desktop.look = "music-desk";
      }
      {
        package = fakeBar // {
          override =
            {
              buildNoDefaultFeatures ? false,
              buildFeatures ? [ ],
            }:
            fakeBar;
        };
        features = [ "clock" ];
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
        # The greeter's session cleanup holds there too, and renders
        # into logind.conf (the freeform `KillOnlyUsers` list as one
        # `greeter` line).
        assert osRealGreeter.config.services.logind.settings.Login.KillUserProcesses == true;
        assert osRealGreeter.config.services.logind.settings.Login.KillOnlyUsers == [ "greeter" ];
        assert lib.hasInfix "KillOnlyUsers=greeter"
          osRealGreeter.config.environment.etc."systemd/logind.conf".text;
        # ...and the switch reload hook rides with it: the unit carries
        # the settings' identity, bound to this evaluation's settings
        # (not a constant), so a switch that changes them HUPs logind.
        assert
          osRealGreeter.config.systemd.services.systemd-logind.unitConfig."X-ScootLogindSettings"
          == builtins.hashString "sha256" (
            builtins.toJSON osRealGreeter.config.services.logind.settings.Login
          );
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
      # The policy's power half in a real NixOS evaluation too: the
      # profiles daemon enabled, low-battery suspend through UPower's
      # own options, the lid actions on the canonical logind path, the
      # charge service, timer and udev rules merged with nixpkgs' own
      # -- and still no default session. (Same `allAssertionsHold`
      # caveat as above.)
      (
        assert osRealPower.config.services.power-profiles-daemon.enable;
        assert osRealPower.config.services.upower.enable;
        assert osRealPower.config.services.upower.percentageAction == 2;
        assert osRealPower.config.services.upower.criticalPowerAction == "Suspend";
        assert osRealPower.config.services.logind.settings.Login.HandleLidSwitch == "suspend";
        assert osRealPower.config.services.logind.settings.Login.HandleLidSwitchDocked == "lock";
        assert osRealPower.config.services.logind.settings.Login.KillUserProcesses == false;
        # ...and no reload hook without the greeter (the identity key
        # rides with the greeter's cleanup, not with the module).
        assert !(osRealPower.config.systemd.services.systemd-logind.unitConfig ? "X-ScootLogindSettings");
        assert osRealPower.config.systemd.services ? scoot-charge-sync;
        assert osRealPower.config.systemd.timers ? scoot-charge-sync;
        assert lib.hasInfix "charge_control_end_threshold" osRealPower.config.services.udev.extraRules;
        assert osRealPower.config.services.displayManager.defaultSession == null;
        assert lib.any (
          p: (p.pname or "") == "power-profiles-daemon"
        ) osRealPower.config.environment.systemPackages;
        true
      )
      # Greeter beside power in a real NixOS evaluation: both halves'
      # logind keys present together (the cleanup pair plus the lid
      # actions) and all rendered into logind.conf -- the two switches
      # compose, neither shadows the other.
      (
        assert osRealGreeterPower.config.services.logind.settings.Login.KillUserProcesses == true;
        assert osRealGreeterPower.config.services.logind.settings.Login.KillOnlyUsers == [ "greeter" ];
        assert osRealGreeterPower.config.services.logind.settings.Login.HandleLidSwitch == "suspend";
        assert osRealGreeterPower.config.services.logind.settings.Login.HandleLidSwitchDocked == "lock";
        assert lib.hasInfix "KillOnlyUsers=greeter"
          osRealGreeterPower.config.environment.etc."systemd/logind.conf".text;
        assert lib.hasInfix "KillUserProcesses=true"
          osRealGreeterPower.config.environment.etc."systemd/logind.conf".text;
        assert lib.hasInfix "HandleLidSwitch=suspend"
          osRealGreeterPower.config.environment.etc."systemd/logind.conf".text;
        assert lib.hasInfix "HandleLidSwitchDocked=lock"
          osRealGreeterPower.config.environment.etc."systemd/logind.conf".text;
        true
      )
      # The documented install configs (desktop/index.md) evaluate with
      # the profile on: the minimal NixOS desktop, the GPU-package plus
      # opt-in-greeter variant, and standalone home-manager. A renamed
      # option or a restructured profile breaks the page's snippets, so
      # it breaks here first.
      (
        assert allAssertionsHold osDocsDesktop.config;
        assert osDocsDesktop.config.programs.scoot.desktop.look == "moonrise";
        assert osDocsDesktop.config.programs.scoot.session.enable;
        true
      )
      (
        assert allAssertionsHold osDocsDesktopFull.config;
        assert osDocsDesktopFull.config.programs.scoot.package.drvPath == built.scoot-gpu.drvPath;
        assert osDocsDesktopFull.config.services.displayManager.regreet.enable;
        true
      )
      (
        assert allAssertionsHold hmDocsDesktop.config;
        assert hmDocsDesktop.config.programs.scoot.desktop.look == "moonrise";
        # Both documented home-manager import spellings resolve to the
        # same module: the current spelling evaluates identically.
        assert hmDocsDesktopCurrent.config.programs.scoot.desktop.look == "moonrise";
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
      # No `scootctl` anywhere in the overlay: the standalone client is
      # gone, `scoot msg` is the only client.
      (
        assert !(overlaid ? scootctl);
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
      # This check run on Darwin itself: no scootbg, and the client-only
      # scoot installed.
      (
        assert hmFlake.config.programs.scoot.wallpaper.package == null;
        true
      )
      (
        assert drvs hmFlake.config.home.packages == drvs [ built.default ];
        true
      )
    ]
    ++ lib.optionals (withFlake && darwinPkgs != null) [
      # A macOS home configuration with a `[wallpaper]` table evaluates,
      # with no scootbg (Linux-only), installing the client-only scoot and
      # rendering the table as written for the Linux box it deploys to.
      (
        assert allAssertionsHold hmDarwin.config;
        true
      )
      (
        assert hmDarwin.config.programs.scoot.wallpaper.package == null;
        true
      )
      (
        assert drvs hmDarwin.config.home.packages == drvs [ flake.packages.aarch64-darwin.default ];
        true
      )
      (
        assert builtins.isString hmDarwin.config.xdg.configFile."scoot/config.toml".source.drvPath;
        true
      )
      # The overlay gives Darwin the client-only scoot, and no scootbg or
      # scootbar (nor any standalone scootctl).
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
        assert darwinOverlaid.scoot.drvPath == flake.packages.aarch64-darwin.scoot.drvPath;
        true
      )
      (
        assert !(darwinOverlaid ? scootctl);
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
  # The single source both `scoot-session.target` installs share: the
  # NixOS side installs these bytes verbatim, the home-manager side
  # sources this file (see `nix/modules/home.nix`).
  scootSessionTargetFile = ../resources/systemd/user/scoot-session.target;
  scootSessionTargetText = builtins.readFile scootSessionTargetFile;
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
  # The documented standalone home-manager desktop renders the moonrise
  # look (site/desktop/index.md's home.nix snippet, end to end).
  hmDocsDesktopToml = hmDocsDesktopPure.config.xdg.configFile."scoot/config.toml".source;
  keysToml = hmKeys.config.xdg.configFile."scoot/config.toml".source;
  keysSlotsToml = hmKeysSlots.config.xdg.configFile."scoot/config.toml".source;
  keysPowerToml = hmPower.config.xdg.configFile."scoot/config.toml".source;
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
  # The notification daemon's generated files: the mako config
  # (overlay layer, DND section, themed leaves, critical ring), plus
  # the lookless, recolored and opt-out variants the content checks
  # read, and the bar file carrying the feed's push module.
  notifConf = hmNotif.config.xdg.configFile."mako/config".source;
  notifNoLookConf = hmNotifNoLook.config.xdg.configFile."mako/config".source;
  notifSettingsConf = hmNotifSettings.config.xdg.configFile."mako/config".source;
  notifTargetOffConf = hmNotifTargetOff.config.xdg.configFile."mako/config".source;
  notifBarToml = hmDeskBarNotif.config.programs.scootbar.configFile;
  notifIconsBarToml = hmDeskBarNotifIcons.config.programs.scootbar.configFile;
  # The clipboard slot's generated files: the swayidle config with the
  # slot off (lock lines without the wipe), plus the store entry and
  # the picker as the units and the keymap run them -- themed (the
  # profile with music-desk), lookless, opt-out, rebound and relocated
  # variants the content checks read.
  idleClipOffConf = hmClipOff.config.xdg.configFile."swayidle/config".source;
  idleClipBoundsConf = hmClipBounds.config.xdg.configFile."swayidle/config".source;
  clipEntry = clipEntryOf hmClip "scoot-clipboard-store";
  clipBoundsEntry = clipEntryOf hmClipBounds "scoot-clipboard-store";
  clipPickerThemed = slotScriptBin hmClip "scoot-clipboard-pick";
  clipBoundsPicker = slotScriptBin hmClipBounds "scoot-clipboard-pick";
  clipPickerNoLook = slotScriptBin hmClipNoLook "scoot-clipboard-pick";
  clipPickerTargetOff = slotScriptBin hmClipTargetOff "scoot-clipboard-pick";
  clipPickerBurst = slotScriptBin hmClipLookBurst "scoot-clipboard-pick";
  # The launcher slot's generated file: the wrapper script as the
  # keymap runs it -- themed (music-desk), lookless, opt-out and one
  # per remaining look (vinyl-sunset, radial-burst, moonrise) the
  # content checks read.
  launchThemed = slotScriptBin hmLaunch "scoot-launcher";
  launchNoLook = slotScriptBin hmLaunchNoLook "scoot-launcher";
  launchTargetOff = slotScriptBin hmLaunchTargetOff "scoot-launcher";
  launchVinyl = slotScriptBin hmLaunchLookVinyl "scoot-launcher";
  launchBurst = slotScriptBin hmLaunchLookBurst "scoot-launcher";
  # The capture slot's generated files: the per-desktop chooser file
  # xdpw reads first -- themed (music-desk), lookless, opt-out, the
  # slurp picker, no picker on a named output and on any output --
  # plus the keymap's three screenshot scripts as their binds run
  # them.
  captureThemed = hmCapture.config.xdg.configFile."xdg-desktop-portal-wlr/scoot".source;
  captureNoLook = hmCaptureNoLook.config.xdg.configFile."xdg-desktop-portal-wlr/scoot".source;
  captureTargetOff = hmCaptureTargetOff.config.xdg.configFile."xdg-desktop-portal-wlr/scoot".source;
  captureSlurp = hmCaptureSlurp.config.xdg.configFile."xdg-desktop-portal-wlr/scoot".source;
  captureNoneNamed = hmCaptureNoneNamed.config.xdg.configFile."xdg-desktop-portal-wlr/scoot".source;
  captureNoneAny = hmCaptureNoneAny.config.xdg.configFile."xdg-desktop-portal-wlr/scoot".source;
  captureOutputScript = slotScriptBin hmCapture "scoot-capture-output";
  captureRegionScript = slotScriptBin hmCapture "scoot-capture-region";
  captureRegionNoLook = slotScriptBin hmCaptureNoLook "scoot-capture-region";
  captureClipboardScript = slotScriptBin hmCapture "scoot-capture-clipboard";
  launchMoon = slotScriptBin hmLaunchLookMoon "scoot-launcher";
  # The power policy's generated files: the switch script as the
  # keymap runs it, the charge script as the service runs it (the
  # `sync` verb stripped), the merged udev rules, and the charge
  # unit, timer and tmpfiles rule the content checks read.
  powerSwitchScript = slotScriptBin hmPower "scoot-power-profile";
  chargeScript = lib.removeSuffix " sync" osPower.config.systemd.services.scoot-charge-sync.serviceConfig.ExecStart;
  chargeService = osPower.config.systemd.services.scoot-charge-sync;
  chargeTimer = osPower.config.systemd.timers.scoot-charge-sync;
  chargeTmpfiles = osPower.config.systemd.tmpfiles.rules;
  chargeUdev = osPower.config.services.udev.extraRules;
  chargeAutoUdev = osPowerAuto.config.services.udev.extraRules;
  # The audio slot's generated files: the wob config -- themed
  # (music-desk), lookless, opt-out -- plus the OSD script and the
  # three control scripts as the keymap's binds run them.
  audioThemed = hmAudio.config.xdg.configFile."wob/wob.ini".source;
  audioNoLook = hmAudioNoLook.config.xdg.configFile."wob/wob.ini".source;
  audioTargetOff = hmAudioTargetOff.config.xdg.configFile."wob/wob.ini".source;
  audioOsdScript = slotScriptBin hmAudio "scoot-osd";
  audioVolumeScript = slotScriptBin hmAudio "scoot-volume";
  audioBrightnessScript = slotScriptBin hmAudio "scoot-brightness";
  audioSinkScript = slotScriptBin hmAudio "scoot-audio-sink";

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
    # ...the greeter's session cleanup: logind kills what the greeter
    # session leaves behind (`KillUserProcesses`), scoped to the
    # greeter user (`KillOnlyUsers`), so other users' lingering
    # processes are untouched.
    (
      assert osGreeter.config.services.logind.settings.Login.KillUserProcesses == true;
      true
    )
    (
      assert osGreeter.config.services.logind.settings.Login.KillOnlyUsers == [ "greeter" ];
      true
    )
    # ...while a user's own `KillUserProcesses` wins over it (a plain
    # assignment beats our `mkDefault`): still evaluates, assertions
    # hold, their value stands.
    (
      assert allAssertionsHold osGreeterLogindOverride.config;
      true
    )
    (
      assert osGreeterLogindOverride.config.services.logind.settings.Login.KillUserProcesses == false;
      true
    )
    # ...and a user's own `KillOnlyUsers` replaces ours (no merge: the
    # list is exactly what they wrote, so dropping `"greeter"` opts
    # the greeter back out of the cleanup -- keep `"greeter"` in the
    # list to keep the fix).
    (
      assert allAssertionsHold osGreeterKillOnlyUsersOverride.config;
      true
    )
    (
      assert
        osGreeterKillOnlyUsersOverride.config.services.logind.settings.Login.KillOnlyUsers == [ "alice" ];
      true
    )
    # ...while the switch reloads logind for the new config: the
    # settings' identity rides in the unit (`X-ScootLogindSettings`),
    # so a switch that only flips these keys HUPs the running logind
    # instead of leaving it on its old in-memory config. The key is
    # bound to the settings (not a constant)...
    (
      assert
        osGreeter.config.systemd.services.systemd-logind.unitConfig."X-ScootLogindSettings"
        == builtins.hashString "sha256" (builtins.toJSON osGreeter.config.services.logind.settings.Login);
      true
    )
    # ...changes when the settings change (either key, greeter's or
    # user's)...
    (
      assert
        osGreeter.config.systemd.services.systemd-logind.unitConfig."X-ScootLogindSettings"
        != osGreeterLogindOverride.config.systemd.services.systemd-logind.unitConfig."X-ScootLogindSettings";
      true
    )
    (
      assert
        osGreeter.config.systemd.services.systemd-logind.unitConfig."X-ScootLogindSettings"
        != osGreeterKillOnlyUsersOverride.config.systemd.services.systemd-logind.unitConfig."X-ScootLogindSettings";
      true
    )
    # ...and is absent with the greeter off (no logind unit touch at
    # all, like the settings pair above).
    (
      assert !(osGreeterOff.config.systemd.services ? systemd-logind);
      true
    )
    # Off changes nothing: no ReGreet, and (no session entry either) no
    # entry and no units -- and no logind change either (the cleanup
    # pair rides with the greeter, not with the module).
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
      assert !(osGreeterOff.config.services.logind.settings.Login ? KillUserProcesses);
      true
    )
    (
      assert !(osGreeterOff.config.services.logind.settings.Login ? KillOnlyUsers);
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
    # ...the clipboard slot on with the profile (the
    # `desktop-clipboard` child): history kept, the tools named, the
    # picker themed unless opted out (without forcing the half-built
    # `theme` slot on)...
    (
      assert hmDesk.config.programs.scoot.desktop.clipboard.enable;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.clipboard.maxItems == 100;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.clipboard.dbPath == null;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.theme.targets.clipboard.enable;
      true
    )
    # ...the launcher on with the profile (fuzzel behind the keymap's
    # binds, themed unless opted out)...
    (
      assert hmDesk.config.programs.scoot.desktop.launcher.enable;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.launcher.daemon == "fuzzel";
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.theme.targets.launcher.enable;
      true
    )
    # ...and every remaining future slot off and empty (spot-check
    # across the tree; `notifications` left this list when the
    # `desktop-notifications` child filled it, `clipboard` when the
    # `desktop-clipboard` child did, `launcher` when the
    # `desktop-launcher` child did, `capture` when the
    # `desktop-capture` child did, `audio` when the
    # `desktop-audio-osd` child did, `nightlight` when the
    # `desktop-nightlight` child did -- `power` stays on it: filled by
    # the `desktop-power` child but opt-in, never with the profile).
    (
      assert hmDesk.config.programs.scoot.desktop.capture.enable;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.capture.chooser == "fuzzel";
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.capture.maxFps == 30;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.capture.outputName == null;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.theme.targets.capture.enable;
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
    # ...the audio slot on with the profile (the
    # `desktop-audio-osd` child): PipeWire running, the OSD on the
    # `overlay` layer behind the keymap's volume, brightness and
    # mic-mute binds (routed through its scripts), the sink helper
    # for the future picker, the OSD themed unless opted out
    # (without forcing the half-built `theme` slot on)...
    (
      assert hmDesk.config.programs.scoot.desktop.audio.enable;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.audio.daemon == "wob";
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.audio.osd.timeoutMs == 1500;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.theme.targets.osd.enable;
      true
    )
    # ...the night light on with the profile (the
    # `desktop-nightlight` child): wlsunset behind the unit, warming
    # on the manual schedule (07:00/19:00, 15 min over, no location
    # needed), themed by the look unless opted out (without forcing
    # the half-built `theme` slot on)...
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.enable;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.daemon == "wlsunset";
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.dayTemp == 6500;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.nightTemp == 3500;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.sunrise == "07:00";
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.sunset == "19:00";
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.duration == 900;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.gamma == 1.0;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.latitude == null;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.nightlight.longitude == null;
      true
    )
    (
      assert hmDesk.config.programs.scoot.desktop.theme.targets.nightlight.enable;
      true
    )
    (
      assert !hmDesk.config.programs.scoot.desktop.power.enable;
      true
    )
    # ...and the charge cap stays off with it (opt-in through the
    # policy, never through the profile).
    (
      assert !hmDesk.config.programs.scoot.desktop.power.chargeLimit.enable;
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
    # ...while the profile turns the keymap on (a laptop whose Fn
    # keys do nothing is not daily-drivable): its binds are the
    # `_keysPins` below.
    (
      assert hmDesk.config.programs.scoot.desktop.keys.enable;
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
    # policy's five tools, the notification daemon, the launcher
    # package and its script, the clipboard slot's three, its picker
    # script, the capture slot's four, the audio slot's OSD and its
    # four scripts, the night light's tool, and the keymap's three,
    # all on with the profile).
    (
      assert hmDeskLookMusic.config.programs.scoot.wallpaper.enable;
      true
    )
    (
      # `brightnessctl` twice is one package, not two tools: the
      # idle policy's dim tool and the keymap's brightness tool are
      # the same derivation, each declared beside its own binds (and
      # `fuzzel` three times the same way: the clipboard picker's
      # menu, the launcher package and the capture chooser's menu --
      # and `wl-clipboard` twice: the clipboard slot's tools and the
      # capture slot's clipboard bind).
      assert
        sorted hmDeskLookMusic.config.home.packages == sorted [
          fakePkg
          fakeBg
          pkgs.swayidle
          pkgs.brightnessctl
          pkgs.wlopm
          pkgs.swaylock
          pkgs.sway-audio-idle-inhibit
          leanMako
          leanClip
          pkgs.wl-clipboard
          pkgs.fuzzel
          (slotScriptDrv hmDeskLookMusic "scoot-clipboard-pick")
          pkgs.fuzzel
          (slotScriptDrv hmDeskLookMusic "scoot-launcher")
          pkgs.grim
          pkgs.slurp
          pkgs.fuzzel
          pkgs.wl-clipboard
          (slotScriptDrv hmDeskLookMusic "scoot-capture-output")
          (slotScriptDrv hmDeskLookMusic "scoot-capture-region")
          (slotScriptDrv hmDeskLookMusic "scoot-capture-clipboard")
          pkgs.wob
          (slotScriptDrv hmDeskLookMusic "scoot-osd")
          (slotScriptDrv hmDeskLookMusic "scoot-volume")
          (slotScriptDrv hmDeskLookMusic "scoot-brightness")
          (slotScriptDrv hmDeskLookMusic "scoot-audio-sink")
          pkgs.wlsunset
          pkgs.brightnessctl
          pkgs.wireplumber
          pkgs.playerctl
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
    # Stylix beside a look wins every leaf: identical to Stylix alone
    # (minus the keymap's `[binds]`, which only the profile side has).
    (
      assert
        builtins.removeAttrs hmDeskStylix.config.programs.scoot.settings [ "binds" ]
        == hmStylix.config.programs.scoot.settings;
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
    # ...the capture slot on with it: the portal service with both
    # backends, the `scoot` backend selection (ScreenCast/Screenshot
    # to `wlr`, the rest to `gtk`), PipeWire running, and the tools
    # on PATH...
    (
      assert osDesk.config.programs.scoot.desktop.capture.enable;
      true
    )
    (
      assert osDesk.config.xdg.portal.enable;
      true
    )
    (
      assert
        osDesk.config.xdg.portal.extraPortals == [
          osDesk.config.programs.scoot.desktop.capture.portalWlrPackage
          osDesk.config.programs.scoot.desktop.capture.portalGtkPackage
        ];
      true
    )
    (
      assert
        osDesk.config.xdg.portal.config.scoot == {
          default = [ "gtk" ];
          "org.freedesktop.impl.portal.Screenshot" = [ "wlr" ];
          "org.freedesktop.impl.portal.ScreenCast" = [ "wlr" ];
        };
      true
    )
    (
      assert osDesk.config.services.pipewire.enable;
      true
    )
    # ...the audio slot on with it: the OSD installed, PipeWire
    # running (the capture slot defaults the same switch, which
    # merges), and `wpctl` on PATH through the keymap's volume tool...
    (
      assert osDesk.config.programs.scoot.desktop.audio.enable;
      true
    )
    (
      assert osDesk.config.programs.scoot.desktop.audio.daemon == "wob";
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "wob") osDesk.config.environment.systemPackages;
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
      assert
        builtins.removeAttrs hmDeskBarStylix.config.programs.scoot.settings [ "binds" ]
        == hmStylix.config.programs.scoot.settings;
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

  # --- session scope structural pins (fail `nix flake check` at eval) ---
  # Linux only: the scope is a user-session wiring, and the units it
  # scopes exist only there (off Linux the slots' tools are null and
  # no unit is written -- pinned in the `_darwin*Pins`).
  _sessionPins = lib.optionals isLinux [
    # The home-manager side installs `scoot-session.target` itself
    # (the NixOS side installs the same file beside its login entry):
    # the same bytes, not a second definition -- `xdg.configFile`
    # sources `resources/systemd/user/scoot-session.target` directly,
    # so the two installs cannot drift. Pinned by content: the
    # installed file reads back identical to the canonical one...
    (
      assert hmIdle.config.xdg.configFile ? "systemd/user/scoot-session.target";
      true
    )
    (
      assert
        builtins.readFile hmIdle.config.xdg.configFile."systemd/user/scoot-session.target".source
        == scootSessionTargetText;
      true
    )
    # ...and the NixOS install is those same bytes too (pinned where
    # the module reads them, so a hand-written copy there fails as
    # well)...
    (
      assert sessionTargetText == scootSessionTargetText;
      true
    )
    # ...whose wiring -- pulling in the shared graphical target,
    # ordered after the pre-session hook -- is pinned here, so the
    # scope's contract is stated where both installs are checked...
    (
      assert lib.hasInfix "Description=scoot session (display ready)" scootSessionTargetText;
      true
    )
    (
      assert lib.hasInfix "BindsTo=graphical-session.target" scootSessionTargetText;
      true
    )
    (
      assert lib.hasInfix "Wants=graphical-session-pre.target" scootSessionTargetText;
      true
    )
    (
      assert lib.hasInfix "After=graphical-session-pre.target" scootSessionTargetText;
      true
    )
    # ...present with the profile (which turns every slot on)...
    (
      assert hmDesk.config.xdg.configFile ? "systemd/user/scoot-session.target";
      true
    )
    # ...present for a standalone slot too (no profile: the slot still
    # runs in the scoot session, never in another desktop's)...
    (
      assert hmIdleStandalone.config.xdg.configFile ? "systemd/user/scoot-session.target";
      true
    )
    (
      assert hmNotifStandalone.config.xdg.configFile ? "systemd/user/scoot-session.target";
      true
    )
    (
      assert hmClipStandalone.config.xdg.configFile ? "systemd/user/scoot-session.target";
      true
    )
    (
      assert hmAudioStandalone.config.xdg.configFile ? "systemd/user/scoot-session.target";
      true
    )
    (
      assert hmNightStandalone.config.xdg.configFile ? "systemd/user/scoot-session.target";
      true
    )
    # ...and absent with nothing to scope (an idle target file with no
    # unit wanting it starts nothing, so none is written).
    (
      assert !(hmOff.config.xdg.configFile ? "systemd/user/scoot-session.target");
      true
    )
    # No profile unit is left on the shared target: besides the four
    # pins above (idle, mako, clipboard, audio), the feed and the
    # profile-managed bar ride the same scope...
    (
      assert hmAudio.config.systemd.user.services.scoot-osd.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmNotif.config.systemd.user.services.scoot-notify-sync.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmDeskBar.config.systemd.user.services.scootbar.Install.WantedBy == [ "scoot-session.target" ];
      true
    )
    (
      assert hmDeskBar.config.systemd.user.services.scootbar.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert hmDeskBar.config.systemd.user.services.scootbar.Unit.After == [ "scoot-session.target" ];
      true
    )
    # ...and on the NixOS side too (the system-wide bar unit beside the
    # session entry, in the profile's look)...
    (
      assert osDeskBar.config.systemd.user.services.scootbar.wantedBy == [ "scoot-session.target" ];
      true
    )
    (
      assert osDeskBar.config.systemd.user.services.scootbar.partOf == [ "scoot-session.target" ];
      true
    )
    (
      assert osDeskBar.config.systemd.user.services.scootbar.after == [ "scoot-session.target" ];
      true
    )
    # A standalone bar stays a generic `graphical-session.target` unit
    # on both sides: with no profile at all...
    (
      assert
        hmBarStandalone.config.systemd.user.services.scootbar.Install.WantedBy
        == [ "graphical-session.target" ];
      true
    )
    (
      assert
        hmBarStandalone.config.systemd.user.services.scootbar.Unit.PartOf == [ "graphical-session.target" ];
      true
    )
    (
      assert
        hmBarStandalone.config.systemd.user.services.scootbar.Unit.After == [ "graphical-session.target" ];
      true
    )
    (
      assert
        osBarStandalone.config.systemd.user.services.scootbar.wantedBy == [ "graphical-session.target" ];
      true
    )
    (
      assert
        osBarStandalone.config.systemd.user.services.scootbar.partOf == [ "graphical-session.target" ];
      true
    )
    (
      assert
        osBarStandalone.config.systemd.user.services.scootbar.after == [ "graphical-session.target" ];
      true
    )
    # ...and beside a profile that leaves the bar alone (the
    # `hmDeskBarOff`/`osDeskBarOff` shape with the user's own bar
    # enabled): still the shared target, so a bar under another
    # compositor starts even where the profile is on.
    (
      assert
        hmDeskBarOffStandalone.config.systemd.user.services.scootbar.Install.WantedBy
        == [ "graphical-session.target" ];
      true
    )
    (
      assert
        hmDeskBarOffStandalone.config.systemd.user.services.scootbar.Unit.PartOf
        == [ "graphical-session.target" ];
      true
    )
    (
      assert
        hmDeskBarOffStandalone.config.systemd.user.services.scootbar.Unit.After
        == [ "graphical-session.target" ];
      true
    )
    (
      assert
        osDeskBarOffStandalone.config.systemd.user.services.scootbar.wantedBy
        == [ "graphical-session.target" ];
      true
    )
    (
      assert
        osDeskBarOffStandalone.config.systemd.user.services.scootbar.partOf
        == [ "graphical-session.target" ];
      true
    )
    (
      assert
        osDeskBarOffStandalone.config.systemd.user.services.scootbar.after
        == [ "graphical-session.target" ];
      true
    )
    # ...while the bar's tray ordering survives the move on both sides
    # (an ordering against an absent unit does nothing).
    (
      assert hmDeskBar.config.systemd.user.services.scootbar.Unit.Before == [ "tray.target" ];
      true
    )
    (
      assert osDeskBar.config.systemd.user.services.scootbar.before == [ "tray.target" ];
      true
    )
  ];

  # --- start-rate-limit placement pins (issue #449) ---
  # systemd reads `StartLimitIntervalSec`/`StartLimitBurst` only in
  # `[Unit]` and logs "Unknown key ... in section [Service]" otherwise,
  # silently keeping the default burst limit (5 starts in 10 s). Every
  # profile unit that retries unendingly names the interval here, so a
  # future unit that puts it in `[Service]` fails `nix flake check` at
  # eval. `hmDeskBar` is the full profile beside the bar module: its
  # services are every user unit the profile can install.
  _startLimitPins = lib.optionals isLinux (
    let
      hmUnits = hmDeskBar.config.systemd.user.services;
      retryUnits = [
        "mako"
        "scoot-notify-sync"
        "scoot-clipboard-store"
        "scoot-clipboard-primary-store"
        "scoot-idle"
        "scoot-audio-inhibit"
        "scoot-osd"
        "scoot-nightlight"
        "scootbar"
      ];
    in
    # The key sits in `[Unit]` on every retrying unit...
    (map (
      name:
      (
        assert hmUnits.${name}.Unit.StartLimitIntervalSec == 0;
        true
      )
    ) retryUnits)
    # ...and in `[Service]` on none of them, nor `StartLimitBurst`
    # in either section...
    ++ (map (
      name:
      (
        assert !(hmUnits.${name}.Service ? StartLimitIntervalSec);
        true
      )
    ) retryUnits)
    ++ (map (
      name:
      (
        assert !(hmUnits.${name}.Service ? StartLimitBurst);
        true
      )
    ) retryUnits)
    ++ (map (
      name:
      (
        assert !(hmUnits.${name}.Unit ? StartLimitBurst);
        true
      )
    ) retryUnits)
    # ...and no other profile unit smuggles either key into its
    # service section either (the sweep: every unit the profile can
    # install, not just the retrying seven)...
    ++ (map (
      name:
      (
        assert !((hmUnits.${name}.Service or { }) ? StartLimitIntervalSec);
        true
      )
    ) (builtins.attrNames hmUnits))
    ++ (map (
      name:
      (
        assert !((hmUnits.${name}.Service or { }) ? StartLimitBurst);
        true
      )
    ) (builtins.attrNames hmUnits))
    # ...and the NixOS bar unit keeps the same placement through
    # `unitConfig` (the NixOS side installs no other profile unit).
    ++ [
      (
        assert osDeskBar.config.systemd.user.services.scootbar.unitConfig.StartLimitIntervalSec == 0;
        true
      )
      (
        assert !(osDeskBar.config.systemd.user.services.scootbar.serviceConfig ? StartLimitIntervalSec);
        true
      )
      (
        assert !(osDeskBar.config.systemd.user.services.scootbar.serviceConfig ? StartLimitBurst);
        true
      )
    ]
  );

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
    # ...bound to scoot's own session scope (`scoot-session.target`,
    # started by the launcher past the display import -- never the
    # shared `graphical-session.target`, which every other desktop
    # reaches too), retried rather than conditioned...
    (
      assert
        hmIdle.config.systemd.user.services.scoot-idle.Install.WantedBy == [ "scoot-session.target" ];
      true
    )
    (
      assert hmIdle.config.systemd.user.services.scoot-idle.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert hmIdle.config.systemd.user.services.scoot-idle.Unit.After == [ "scoot-session.target" ];
      true
    )
    # ...and the audio inhibitor bound to the same scope...
    (
      assert
        hmIdle.config.systemd.user.services.scoot-audio-inhibit.Install.WantedBy
        == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmIdle.config.systemd.user.services.scoot-audio-inhibit.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmIdle.config.systemd.user.services.scoot-audio-inhibit.Unit.After == [ "scoot-session.target" ];
      true
    )
    # ...waiting for each command (the before-sleep lock lands before
    # logind sleeps) from the generated config...
    (
      assert lib.hasInfix "/bin/swayidle -w -C "
        hmIdle.config.systemd.user.services.scoot-idle.Service.ExecStart;
      true
    )
    # ...exactly the twenty-three packages installed (swayidle, dim,
    # off, locker, inhibitor, mako, the clipboard slot's three, its
    # picker script, the launcher package and its script, the capture
    # slot's four tools and its three scripts, the night light's tool
    # -- no scoot package
    # set here, so nothing else -- plus the keymap's brightness,
    # volume and media tools; `brightnessctl` and `fuzzel` each serve
    # two features, so each appears twice).
    (
      assert builtins.length hmIdle.config.home.packages == 28;
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
          leanMako
          leanClip
          pkgs.wl-clipboard
          pkgs.fuzzel
          (slotScriptDrv hmIdle "scoot-clipboard-pick")
          pkgs.fuzzel
          (slotScriptDrv hmIdle "scoot-launcher")
          pkgs.grim
          pkgs.slurp
          pkgs.fuzzel
          pkgs.wl-clipboard
          (slotScriptDrv hmIdle "scoot-capture-output")
          (slotScriptDrv hmIdle "scoot-capture-region")
          (slotScriptDrv hmIdle "scoot-capture-clipboard")
          pkgs.wob
          (slotScriptDrv hmIdle "scoot-osd")
          (slotScriptDrv hmIdle "scoot-volume")
          (slotScriptDrv hmIdle "scoot-brightness")
          (slotScriptDrv hmIdle "scoot-audio-sink")
          pkgs.wlsunset
          pkgs.brightnessctl
          pkgs.wireplumber
          pkgs.playerctl
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
    # The policy off: no units, no files beyond the profile's own --
    # but the notification daemon stays (its switch is its own, on
    # with the profile), the clipboard slot stays too (its switch is
    # its own as well) and the keymap stays too (the other child,
    # still on with the profile), so mako, the clipboard watchers and
    # their tools stay.
    (
      assert allAssertionsHold hmIdleOff.config;
      true
    )
    (
      assert
        builtins.attrNames hmIdleOff.config.systemd.user.services == [
          "mako"
          "scoot-clipboard-primary-store"
          "scoot-clipboard-store"
          "scoot-nightlight"
          "scoot-notify-sync"
          "scoot-osd"
        ];
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
      assert hmIdleOff.config.xdg.configFile ? "mako/config";
      true
    )
    (
      assert
        sorted hmIdleOff.config.home.packages == sorted [
          leanMako
          leanClip
          pkgs.wl-clipboard
          pkgs.fuzzel
          (slotScriptDrv hmIdleOff "scoot-clipboard-pick")
          pkgs.fuzzel
          (slotScriptDrv hmIdleOff "scoot-launcher")
          pkgs.grim
          pkgs.slurp
          pkgs.fuzzel
          pkgs.wl-clipboard
          (slotScriptDrv hmIdleOff "scoot-capture-output")
          (slotScriptDrv hmIdleOff "scoot-capture-region")
          (slotScriptDrv hmIdleOff "scoot-capture-clipboard")
          pkgs.wob
          (slotScriptDrv hmIdleOff "scoot-osd")
          (slotScriptDrv hmIdleOff "scoot-volume")
          (slotScriptDrv hmIdleOff "scoot-brightness")
          (slotScriptDrv hmIdleOff "scoot-audio-sink")
          pkgs.wlsunset
          pkgs.brightnessctl
          pkgs.wireplumber
          pkgs.playerctl
        ];
      true
    )
    # The lock off: the policy stays (dim and screens-off), the locker
    # leaves (no config, no package, four policy tools plus mako, the
    # clipboard slot's three, its picker script, the launcher package
    # and its script, the capture slot's four, the audio slot's OSD
    # and its four scripts, the night light's tool, and the keymap's
    # three
    # left).
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
      assert builtins.length hmLockOff.config.home.packages == 27;
      true
    )
    # The inhibitor off: the policy without the audio hold (four
    # policy tools plus mako, the clipboard slot's three, its picker
    # script, the launcher package and its script, the capture slot's
    # four, the audio slot's OSD and its four scripts, the night
    # light's tool, and the keymap's three).
    (
      assert allAssertionsHold hmInhibitOff.config;
      true
    )
    (
      assert !(hmInhibitOff.config.systemd.user.services ? scoot-audio-inhibit);
      true
    )
    (
      assert builtins.length hmInhibitOff.config.home.packages == 27;
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

    # NixOS: the profile installs the five policy tools, mako, the
    # launcher, the clipboard slot's three, the capture slot's five
    # and the keymap's three
    # beside scoot and scootbg, locks docked lids, and names the
    # locker's PAM service -- staying additive (no default session,
    # ever)...
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
          leanMako
          pkgs.fuzzel
          leanClip
          pkgs.wl-clipboard
          pkgs.fuzzel
          pkgs.xdg-desktop-portal-wlr
          pkgs.xdg-desktop-portal-gtk
          pkgs.grim
          pkgs.slurp
          pkgs.fuzzel
          pkgs.wob
          pkgs.pipewire
          pkgs.wlsunset
          pkgs.brightnessctl
          pkgs.wireplumber
          pkgs.playerctl
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
    # applies), no PAM -- but the daemon stays (its switch is its
    # own), the clipboard slot stays too (its switch is its own as
    # well), the launcher stays too and the keymap stays too (the
    # other child), so mako, the launcher, the clipboard tools and the
    # keymap's three stay...
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
          leanMako
          pkgs.fuzzel
          leanClip
          pkgs.wl-clipboard
          pkgs.fuzzel
          pkgs.xdg-desktop-portal-wlr
          pkgs.xdg-desktop-portal-gtk
          pkgs.grim
          pkgs.slurp
          pkgs.fuzzel
          pkgs.wob
          pkgs.pipewire
          pkgs.wlsunset
          pkgs.brightnessctl
          pkgs.wireplumber
          pkgs.playerctl
        ];
      true
    )
    # ...the lock off: no PAM and no locker, the lid rule still
    # locking (dim and screens-off still run from the home-manager
    # side, the launcher, the clipboard tools and the keymap's tools
    # beside them)...
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
          leanMako
          pkgs.fuzzel
          leanClip
          pkgs.wl-clipboard
          pkgs.fuzzel
          pkgs.xdg-desktop-portal-wlr
          pkgs.xdg-desktop-portal-gtk
          pkgs.grim
          pkgs.slurp
          pkgs.fuzzel
          pkgs.wob
          pkgs.pipewire
          pkgs.wlsunset
          pkgs.brightnessctl
          pkgs.wireplumber
          pkgs.playerctl
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

  # --- notification daemon structural pins (fail `nix flake check` at eval) ---
  # Linux only, like the policy above: every evaluation here runs the
  # daemon, whose package refuses evaluation on Darwin (the null
  # degradation itself is pinned in `_darwinNotifPins`).
  _notifPins = lib.optionals isLinux [
    # Home-manager: the whole slot on (units, file, tool beside the
    # profile's own)...
    (
      assert allAssertionsHold hmNotif.config;
      true
    )
    (
      assert hmNotif.config.programs.scoot.desktop.notifications.enable;
      true
    )
    (
      assert hmNotif.config.programs.scoot.desktop.notifications.daemon == "mako";
      true
    )
    # ...running the lean mako (no GTK stack), not stock nixpkgs mako:
    # the default is the very derivation `leanMako` names (same `drvPath`,
    # so a revert to `pkgs.mako` fails here), hence a different store
    # path than stock...
    (
      assert hmNotif.config.programs.scoot.desktop.notifications.package.drvPath == leanMako.drvPath;
      true
    )
    (
      assert hmNotif.config.programs.scoot.desktop.notifications.package.outPath != pkgs.mako.outPath;
      true
    )
    # ...and the NixOS side defaults to the same derivation (either
    # side alone names the same daemon)...
    (
      assert osNotif.config.programs.scoot.desktop.notifications.package.drvPath == leanMako.drvPath;
      true
    )
    (
      assert hmNotif.config.systemd.user.services ? mako;
      true
    )
    (
      assert hmNotif.config.systemd.user.services ? scoot-notify-sync;
      true
    )
    (
      assert hmNotif.config.xdg.configFile ? "mako/config";
      true
    )
    # ...the daemon bound to scoot's own session scope (started by the
    # launcher past the display import -- never the shared
    # `graphical-session.target`, which every other desktop reaches
    # too), retried rather than conditioned...
    (
      assert hmNotif.config.systemd.user.services.mako.Install.WantedBy == [ "scoot-session.target" ];
      true
    )
    (
      assert hmNotif.config.systemd.user.services.mako.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert hmNotif.config.systemd.user.services.mako.Unit.After == [ "scoot-session.target" ];
      true
    )
    # ...activatable over D-Bus (a `Notify` with the daemon down
    # starts the unit through mako's activation file), running this
    # package's binary, reloaded through its `makoctl`...
    (
      assert hmNotif.config.systemd.user.services.mako.Service.Type == "dbus";
      true
    )
    (
      assert hmNotif.config.systemd.user.services.mako.Service.BusName == "org.freedesktop.Notifications";
      true
    )
    (
      assert lib.hasInfix "/bin/mako" hmNotif.config.systemd.user.services.mako.Service.ExecStart;
      true
    )
    (
      assert lib.hasInfix "/bin/makoctl reload"
        hmNotif.config.systemd.user.services.mako.Service.ExecReload;
      true
    )
    # ...gated on the display (an activation before the session
    # reaches the graphical target skips cleanly instead of spinning
    # restarts)...
    (
      assert lib.hasInfix "WAYLAND_DISPLAY"
        hmNotif.config.systemd.user.services.mako.Service.ExecCondition;
      true
    )
    # ...and the feed ordered after it, wanted by (and stopped with)
    # the same scope.
    (
      assert
        hmNotif.config.systemd.user.services.scoot-notify-sync.Install.WantedBy
        == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmNotif.config.systemd.user.services.scoot-notify-sync.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmNotif.config.systemd.user.services.scoot-notify-sync.Unit.After == [
          "scoot-session.target"
          "mako.service"
        ];
      true
    )
    (
      assert lib.hasInfix "scoot-notify-sync --watch"
        hmNotif.config.systemd.user.services.scoot-notify-sync.Service.ExecStart;
      true
    )
    # Without a look the daemon runs unthemed (the units and the file
    # are still there; mako keeps its own colors -- pinned by content
    # below).
    (
      assert allAssertionsHold hmNotifNoLook.config;
      true
    )
    (
      assert hmNotifNoLook.config.systemd.user.services ? mako;
      true
    )
    (
      assert hmNotifNoLook.config.systemd.user.services ? scoot-notify-sync;
      true
    )
    (
      assert hmNotifNoLook.config.xdg.configFile ? "mako/config";
      true
    )
    # The daemon off: no units, no file beyond the profile's own, no
    # tool (the idle policy stays: this switch is its own).
    (
      assert allAssertionsHold hmNotifOff.config;
      true
    )
    (
      assert !(hmNotifOff.config.systemd.user.services ? mako);
      true
    )
    (
      assert !(hmNotifOff.config.systemd.user.services ? scoot-notify-sync);
      true
    )
    (
      assert !(hmNotifOff.config.xdg.configFile ? "mako/config");
      true
    )
    (
      assert hmNotifOff.config.systemd.user.services ? scoot-idle;
      true
    )
    # Standalone (no profile): the daemon runs, unthemed.
    (
      assert allAssertionsHold hmNotifStandalone.config;
      true
    )
    (
      assert hmNotifStandalone.config.systemd.user.services ? mako;
      true
    )
    (
      assert hmNotifStandalone.config.systemd.user.services ? scoot-notify-sync;
      true
    )
    (
      assert hmNotifStandalone.config.xdg.configFile ? "mako/config";
      true
    )
    # Rethemed, rebound and recolored: every assertion still holds
    # (the content checks below prove the values land).
    (
      assert allAssertionsHold hmNotifSettings.config;
      true
    )
    (
      assert allAssertionsHold hmNotifTargetOff.config;
      true
    )
    # Refusals: the daemon with no mako to run it...
    (
      assert builtins.length (failing hmNotifNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "notifications.package is null" (builtins.head (failing hmNotifNoPkg.config));
      true
    )
    # ...and an unknown daemon, an enum type error (verified by hand
    # to name the one valid value).
    (
      assert !hmNotifDaemonBogus.success;
      true
    )

    # NixOS: the profile installs mako beside scoot and scootbg --
    # staying additive (no default session, ever)...
    (
      assert allAssertionsHold osNotif.config;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "mako") osNotif.config.environment.systemPackages;
      true
    )
    (
      assert osNotif.config.services.displayManager.defaultSession == null;
      true
    )
    # ...the daemon off: the profile's own packages only...
    (
      assert allAssertionsHold osNotifOff.config;
      true
    )
    (
      assert !(lib.any (p: (p.pname or "") == "mako") osNotifOff.config.environment.systemPackages);
      true
    )
    # ...and the refusal naming the daemon on this side as well.
    (
      assert builtins.length (failing osNotifNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "notifications.package is null" (builtins.head (failing osNotifNoPkg.config));
      true
    )

    # With the bar module: the feed's `push` module is defined, with
    # the DND toggle as its click (the daemon's own command, by
    # absolute path)...
    (
      assert allAssertionsHold hmDeskBarNotif.config;
      true
    )
    (
      assert
        hmDeskBarNotif.config.programs.scootbar.settings.push.notifications.on-click.exec == [
          "${leanMako}/bin/makoctl"
          "mode"
          "-t"
          "do-not-disturb"
        ];
      true
    )
    # ...a user's own click winning per leaf (pinned structurally:
    # the whole table is defined leaf by leaf, the way the colors
    # are)...
    (
      assert
        (evalHomeDesktopWith [ ]
          {
            enable = true;
            package = fakePkg;
            wallpaper.package = fakeBg;
            desktop.enable = true;
            desktop.look = "music-desk";
          }
          {
            package = fakeBar;
            settings.push.notifications.on-click.exec = [ "true" ];
          }
        ).config.programs.scootbar.settings.push.notifications.on-click.exec == [ "true" ];
      true
    )
    # ...and on NixOS too, in that side's look.
    (
      assert allAssertionsHold osDeskBar.config;
      true
    )
    (
      assert
        osDeskBar.config.programs.scootbar.settings.push.notifications.on-click.exec == [
          "${leanMako}/bin/makoctl"
          "mode"
          "-t"
          "do-not-disturb"
        ];
      true
    )
    # The state icons default to DejaVu Sans glyphs (hollow circle for
    # idle, solid dot for unread, crescent moon for DND)...
    (
      assert
        hmNotif.config.programs.scoot.desktop.notifications.bar.icons == {
          idle = "○";
          unread = "●";
          dnd = "☾";
        };
      true
    )
    # ...the bar's static icon is the idle one...
    (
      assert hmDeskBarNotif.config.programs.scootbar.settings.push.notifications.icon == "○";
      true
    )
    # ...each key overridable (custom idle reaches the bar, empty DND
    # reaches the option the feed reads)...
    (
      assert hmDeskBarNotifIcons.config.programs.scootbar.settings.push.notifications.icon == "✉";
      true
    )
    (
      assert hmDeskBarNotifIcons.config.programs.scoot.desktop.notifications.bar.icons.unread == "!";
      true
    )
    (
      assert hmDeskBarNotifIcons.config.programs.scoot.desktop.notifications.bar.icons.dnd == "";
      true
    )
    # ...an empty idle sets no static icon at all (a quiet desktop shows
    # nothing)...
    (
      assert
        !(
          (evalHomeDesktopWith [ ] {
            enable = true;
            package = fakePkg;
            wallpaper.package = fakeBg;
            desktop.enable = true;
            desktop.look = "music-desk";
            desktop.notifications.bar.icons.idle = "";
          } { package = fakeBar; }).config.programs.scootbar.settings.push.notifications ? icon
        );
      true
    )
    # ...and a two-glyph icon is refused at eval, naming the option
    # (the bar refuses it per update at runtime).
    (
      assert
        builtins.length (
          failing
            (evalHome {
              enable = true;
              desktop.notifications.enable = true;
              desktop.notifications.bar.icons.dnd = "ab";
            }).config
        ) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "bar.icons" m) (
        failing
          (evalHome {
            enable = true;
            desktop.notifications.enable = true;
            desktop.notifications.bar.icons.dnd = "ab";
          }).config
      );
      true
    )
    # With the daemon off, or the bar unmanaged, the table stays
    # absent (the bar is entirely the user's then: no feed, no
    # toggle).
    (
      assert (hmDeskBarNoNotif.config.programs.scootbar.settings.push or { }) == { };
      true
    )
    (
      assert (hmDeskBarOff.config.programs.scootbar.settings.push or { }) == { };
      true
    )
    (
      assert (osDeskBarOff.config.programs.scootbar.settings.push or { }) == { };
      true
    )
    # With a bar built without the push module: exactly one failing
    # assertion, naming it (the bar itself would refuse the table at
    # startup).
    (
      assert builtins.length (failing hmDeskBarNoPush.config) == 1;
      true
    )
    (
      assert lib.hasInfix "needs the push" (builtins.head (failing hmDeskBarNoPush.config));
      true
    )
  ];

  # --- clipboard slot structural pins (fail `nix flake check` at eval) ---
  # Linux only, like the daemon above: every evaluation here runs the
  # slot, whose tools refuse evaluation on Darwin (the null degradation
  # itself is pinned in `_darwinClipPins`).
  _clipPins = lib.optionals isLinux [
    # Home-manager: the whole slot on (two watcher units, three tools
    # beside the profile's own)...
    (
      assert allAssertionsHold hmClip.config;
      true
    )
    (
      assert hmClip.config.programs.scoot.desktop.clipboard.enable;
      true
    )
    (
      assert hmClip.config.programs.scoot.desktop.clipboard.maxItems == 100;
      true
    )
    (
      assert hmClip.config.programs.scoot.desktop.clipboard.dbPath == null;
      true
    )
    # ...running the lean cliphist (no contrib pickers), not stock
    # nixpkgs cliphist: the default is the very derivation `leanClip`
    # names (same `drvPath`, so a revert to `pkgs.cliphist` fails
    # here), hence a different store path than stock...
    (
      assert hmClip.config.programs.scoot.desktop.clipboard.managerPackage.drvPath == leanClip.drvPath;
      true
    )
    (
      assert
        hmClip.config.programs.scoot.desktop.clipboard.managerPackage.outPath != pkgs.cliphist.outPath;
      true
    )
    # ...and the other two tools stock from nixpkgs...
    (
      assert
        hmClip.config.programs.scoot.desktop.clipboard.wlClipboardPackage.drvPath
        == pkgs.wl-clipboard.drvPath;
      true
    )
    (
      assert hmClip.config.programs.scoot.desktop.clipboard.menuPackage.drvPath == pkgs.fuzzel.drvPath;
      true
    )
    # ...and the NixOS side defaults to the same three derivations
    # (either side alone names the same tools)...
    (
      assert osClip.config.programs.scoot.desktop.clipboard.managerPackage.drvPath == leanClip.drvPath;
      true
    )
    (
      assert
        osClip.config.programs.scoot.desktop.clipboard.wlClipboardPackage.drvPath
        == pkgs.wl-clipboard.drvPath;
      true
    )
    (
      assert osClip.config.programs.scoot.desktop.clipboard.menuPackage.drvPath == pkgs.fuzzel.drvPath;
      true
    )
    (
      assert hmClip.config.systemd.user.services ? scoot-clipboard-store;
      true
    )
    (
      assert hmClip.config.systemd.user.services ? scoot-clipboard-primary-store;
      true
    )
    # ...both watchers bound to scoot's own session scope (started by
    # the launcher past the display import -- never the shared
    # `graphical-session.target`, which every other desktop reaches
    # too), retried rather than conditioned...
    (
      assert
        hmClip.config.systemd.user.services.scoot-clipboard-store.Install.WantedBy
        == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmClip.config.systemd.user.services.scoot-clipboard-store.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmClip.config.systemd.user.services.scoot-clipboard-store.Unit.After == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmClip.config.systemd.user.services.scoot-clipboard-primary-store.Install.WantedBy
        == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmClip.config.systemd.user.services.scoot-clipboard-primary-store.Unit.PartOf
        == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmClip.config.systemd.user.services.scoot-clipboard-primary-store.Unit.After
        == [ "scoot-session.target" ];
      true
    )
    # ...watching through `wl-paste` (this package's binary, absolute so
    # it works off PATH), the primary watcher with `--primary`, each
    # running the store entry...
    (
      assert contains "${pkgs.wl-clipboard}/bin/wl-paste --watch"
        hmClip.config.systemd.user.services.scoot-clipboard-store.Service.ExecStart;
      true
    )
    (
      assert contains "${pkgs.wl-clipboard}/bin/wl-paste --primary --watch"
        hmClip.config.systemd.user.services.scoot-clipboard-primary-store.Service.ExecStart;
      true
    )
    (
      assert lib.hasSuffix "/bin/scoot-clipboard-store-entry" (
        clipEntryOf hmClip "scoot-clipboard-store"
      );
      true
    )
    # ...gated on the display (a start before the session reaches the
    # graphical target skips cleanly instead of spinning restarts)...
    (
      assert lib.hasInfix "WAYLAND_DISPLAY"
        hmClip.config.systemd.user.services.scoot-clipboard-store.Service.ExecCondition;
      true
    )
    # Without a look the slot runs unthemed (the units and the tools are
    # still there; the picker keeps fuzzel's own colors -- pinned by
    # content below).
    (
      assert allAssertionsHold hmClipNoLook.config;
      true
    )
    (
      assert hmClipNoLook.config.systemd.user.services ? scoot-clipboard-store;
      true
    )
    (
      assert hmClipNoLook.config.systemd.user.services ? scoot-clipboard-primary-store;
      true
    )
    # The slot off: no units, no tools beyond the profile's own (the
    # idle policy stays: this switch is its own).
    (
      assert allAssertionsHold hmClipOff.config;
      true
    )
    (
      assert !(hmClipOff.config.systemd.user.services ? scoot-clipboard-store);
      true
    )
    (
      assert !(hmClipOff.config.systemd.user.services ? scoot-clipboard-primary-store);
      true
    )
    (
      assert !(lib.any (p: (p.pname or "") == "cliphist") hmClipOff.config.home.packages);
      true
    )
    # ...`wl-clipboard` stays: the capture slot (on with the profile)
    # runs its clipboard bind from the same derivation -- and with
    # both slots off it leaves entirely...
    (
      assert lib.any (p: (p.pname or "") == "wl-clipboard") hmClipOff.config.home.packages;
      true
    )
    (
      assert !(lib.any (p: (p.pname or "") == "wl-clipboard") hmClipCaptureOff.config.home.packages);
      true
    )
    # ...fuzzel stays: the launcher slot (on with the profile) runs
    # the same derivation.
    (
      assert lib.any (p: (p.pname or "") == "fuzzel") hmClipOff.config.home.packages;
      true
    )
    (
      assert hmClipOff.config.systemd.user.services ? scoot-idle;
      true
    )
    # Standalone (no profile): the slot runs, unthemed.
    (
      assert allAssertionsHold hmClipStandalone.config;
      true
    )
    (
      assert hmClipStandalone.config.systemd.user.services ? scoot-clipboard-store;
      true
    )
    (
      assert hmClipStandalone.config.systemd.user.services ? scoot-clipboard-primary-store;
      true
    )
    # Rebounded, relocated and re-looked: every assertion still holds
    # (the content checks below prove the values land in the entry
    # script and the picker).
    (
      assert allAssertionsHold hmClipBounds.config;
      true
    )
    (
      assert allAssertionsHold hmClipTargetOff.config;
      true
    )
    (
      assert allAssertionsHold hmClipLookBurst.config;
      true
    )
    # Refusals: the slot with each tool missing...
    (
      assert builtins.length (failing hmClipNoManager.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.managerPackage is null" (
        builtins.head (failing hmClipNoManager.config)
      );
      true
    )
    (
      assert builtins.length (failing hmClipNoTools.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.wlClipboardPackage is null" (
        builtins.head (failing hmClipNoTools.config)
      );
      true
    )
    (
      assert builtins.length (failing hmClipNoMenu.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.menuPackage is null" (builtins.head (failing hmClipNoMenu.config));
      true
    )
    # ...a history of nothing, and a db path in each refused shape
    # (`~`, relative, space, quote, `$`, backtick, `;`, empty -- each
    # naming a file the three render sites would disagree on)...
    (
      assert builtins.length (failing hmClipBadMax.config) == 1;
      true
    )
    (
      assert lib.hasInfix "maxItems" (builtins.head (failing hmClipBadMax.config));
      true
    )
    (
      assert builtins.length (failing hmClipBadDbTilde.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.dbPath" (builtins.head (failing hmClipBadDbTilde.config));
      true
    )
    (
      assert builtins.length (failing hmClipBadDbRelative.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.dbPath" (builtins.head (failing hmClipBadDbRelative.config));
      true
    )
    (
      assert builtins.length (failing hmClipBadDbSpace.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.dbPath" (builtins.head (failing hmClipBadDbSpace.config));
      true
    )
    (
      assert builtins.length (failing hmClipBadDbQuote.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.dbPath" (builtins.head (failing hmClipBadDbQuote.config));
      true
    )
    (
      assert builtins.length (failing hmClipBadDbDollar.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.dbPath" (builtins.head (failing hmClipBadDbDollar.config));
      true
    )
    (
      assert builtins.length (failing hmClipBadDbBacktick.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.dbPath" (builtins.head (failing hmClipBadDbBacktick.config));
      true
    )
    (
      assert builtins.length (failing hmClipBadDbSemi.config) == 1;
      true
    )
    (
      assert builtins.length (failing hmClipBadDbGlob.config) == 1;
      true
    )
    (
      assert builtins.length (failing hmClipBadDbPipe.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.dbPath" (builtins.head (failing hmClipBadDbSemi.config));
      true
    )
    (
      assert builtins.length (failing hmClipBadDbEmpty.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.dbPath" (builtins.head (failing hmClipBadDbEmpty.config));
      true
    )

    # NixOS: the profile installs the three tools beside scoot and
    # scootbg -- staying additive (no default session, ever)...
    (
      assert allAssertionsHold osClip.config;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "cliphist") osClip.config.environment.systemPackages;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "wl-clipboard") osClip.config.environment.systemPackages;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "fuzzel") osClip.config.environment.systemPackages;
      true
    )
    (
      assert osClip.config.services.displayManager.defaultSession == null;
      true
    )
    # ...the slot off: the profile's own packages only...
    (
      assert allAssertionsHold osClipOff.config;
      true
    )
    (
      assert !(lib.any (p: (p.pname or "") == "cliphist") osClipOff.config.environment.systemPackages);
      true
    )
    (
      assert
        !(lib.any (p: (p.pname or "") == "wl-clipboard") osClipOff.config.environment.systemPackages);
      true
    )
    # ...fuzzel stays: the launcher slot (on with the profile) runs
    # the same derivation.
    (
      assert lib.any (p: (p.pname or "") == "fuzzel") osClipOff.config.environment.systemPackages;
      true
    )
    # ...and each refusal naming its tool on this side as well.
    (
      assert builtins.length (failing osClipNoManager.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.managerPackage is null" (
        builtins.head (failing osClipNoManager.config)
      );
      true
    )
    (
      assert builtins.length (failing osClipNoTools.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.wlClipboardPackage is null" (
        builtins.head (failing osClipNoTools.config)
      );
      true
    )
    (
      assert builtins.length (failing osClipNoMenu.config) == 1;
      true
    )
    (
      assert lib.hasInfix "clipboard.menuPackage is null" (builtins.head (failing osClipNoMenu.config));
      true
    )
  ];

  # --- launcher structural pins (fail `nix flake check` at eval) ---
  # Linux only, like the slots above: the package refuses evaluation
  # on Darwin (the null degradation itself is pinned in
  # `_darwinLaunchPins`).
  _launchPins = lib.optionals isLinux [
    # Home-manager: the whole slot on (the package beside the
    # profile's own, the wrapper script beside the keymap's binds)...
    (
      assert allAssertionsHold hmLaunch.config;
      true
    )
    (
      assert hmLaunch.config.programs.scoot.desktop.launcher.enable;
      true
    )
    (
      assert hmLaunch.config.programs.scoot.desktop.launcher.daemon == "fuzzel";
      true
    )
    # ...running stock nixpkgs fuzzel (the same derivation the
    # clipboard picker themes: one fuzzel, no second copy -- the same
    # `drvPath` on both sides, so a divergent package fails here)...
    (
      assert hmLaunch.config.programs.scoot.desktop.launcher.package.drvPath == pkgs.fuzzel.drvPath;
      true
    )
    (
      assert
        hmLaunch.config.programs.scoot.desktop.launcher.package.drvPath
        == hmLaunch.config.programs.scoot.desktop.clipboard.menuPackage.drvPath;
      true
    )
    (
      assert osLaunch.config.programs.scoot.desktop.launcher.package.drvPath == pkgs.fuzzel.drvPath;
      true
    )
    # ...installed beside the profile's own (NixOS: system-wide)...
    (
      assert lib.any (p: (p.pname or "") == "fuzzel") hmLaunch.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.name or "") == "scoot-launcher") hmLaunch.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "fuzzel") osLaunch.config.environment.systemPackages;
      true
    )
    # ...both binds through the wrapper script (drun by default, PATH
    # executables on the run bind)...
    (
      assert
        hmLaunch.config.programs.scoot.settings.binds."super+d"
        == "spawn ${slotScriptBin hmLaunch "scoot-launcher"}";
      true
    )
    (
      assert
        hmLaunch.config.programs.scoot.settings.binds."ctrl+alt+space"
        == "spawn ${slotScriptBin hmLaunch "scoot-launcher"} --list-executables-in-path";
      true
    )
    # Without a look the slot runs unthemed (the binary and the binds
    # are still there; the launcher keeps fuzzel's own colors --
    # pinned by content below).
    (
      assert allAssertionsHold hmLaunchNoLook.config;
      true
    )
    (
      assert hmLaunchNoLook.config.programs.scoot.settings.binds ? "super+d";
      true
    )
    (
      assert hmLaunchNoLook.config.programs.scoot.settings.binds ? "ctrl+alt+space";
      true
    )
    # The slot off: no package, no binds, no script beyond the
    # profile's own (the idle policy stays: this switch is its own).
    (
      assert allAssertionsHold hmLaunchOff.config;
      true
    )
    (
      assert !(hmLaunchOff.config.programs.scoot.settings.binds ? "super+d");
      true
    )
    (
      assert !(hmLaunchOff.config.programs.scoot.settings.binds ? "ctrl+alt+space");
      true
    )
    # ...fuzzel stays: the clipboard picker (on with the profile) runs
    # the same derivation -- but the launcher script leaves with the
    # slot.
    (
      assert lib.any (p: (p.pname or "") == "fuzzel") hmLaunchOff.config.home.packages;
      true
    )
    (
      assert !(lib.any (p: (p.name or "") == "scoot-launcher") hmLaunchOff.config.home.packages);
      true
    )
    (
      assert hmLaunchOff.config.systemd.user.services ? scoot-idle;
      true
    )
    (
      assert allAssertionsHold osLaunchOff.config;
      true
    )
    # ...fuzzel stays system-wide too: the clipboard slot (on with the
    # profile) installs the same derivation.
    (
      assert lib.any (p: (p.pname or "") == "fuzzel") osLaunchOff.config.environment.systemPackages;
      true
    )
    # Standalone (no profile): the binary on PATH, unthemed, no binds
    # (those need the keymap).
    (
      assert allAssertionsHold hmLaunchStandalone.config;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "fuzzel") hmLaunchStandalone.config.home.packages;
      true
    )
    (
      assert !(hmLaunchStandalone.config.programs.scoot.settings ? binds);
      true
    )
    # Re-looked and opted out: every assertion still holds (the
    # content checks below prove the flags land in the script).
    (
      assert allAssertionsHold hmLaunchTargetOff.config;
      true
    )
    (
      assert allAssertionsHold hmLaunchLookVinyl.config;
      true
    )
    (
      assert allAssertionsHold hmLaunchLookBurst.config;
      true
    )
    (
      assert allAssertionsHold hmLaunchLookMoon.config;
      true
    )
    # Refusals: the slot with no launcher to run it...
    (
      assert builtins.length (failing hmLaunchNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "launcher.package is null" (builtins.head (failing hmLaunchNoPkg.config));
      true
    )
    (
      assert builtins.length (failing osLaunchNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "launcher.package is null" (builtins.head (failing osLaunchNoPkg.config));
      true
    )
    # ...and an unknown daemon, which is an option type error (the
    # `enum`'s own message names the valid value), caught here by
    # `tryEval`.
    (
      assert !hmLaunchDaemonBogus.success;
      true
    )
  ];

  # --- launcher off Linux (fail `nix flake check` at eval) ---
  #
  # fuzzel is Linux-only: off Linux its package defaults to null,
  # which the slot's own assertion refuses loudly instead of
  # installing nothing silently. Empty off Linux (the Linux check
  # above is where the slot is pinned).
  _darwinLaunchPins = lib.optionals (!isLinux) [
    # Home-manager: null, and the slot's assertion refusing loudly
    # (the idle policy's five plus the daemon's one plus the
    # launcher's one plus the clipboard slot's three plus the capture
    # slot's four plus the audio slot's two plus the night light's
    # one: the profile is
    # on in this evaluation, so its slot is open).
    (
      assert hmLaunch.config.programs.scoot.desktop.launcher.package == null;
      true
    )
    (
      assert builtins.length (failing hmLaunch.config) == 17;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "launcher.package is null" m) (failing hmLaunch.config);
      true
    )
    # NixOS: the same null (no launcher installed)...
    (
      assert osLaunch.config.programs.scoot.desktop.launcher.package == null;
      true
    )
    # ...refused loudly there too.
    (
      assert builtins.length (failing osLaunch.config) == 18;
      true
    )
  ];

  # --- capture slot (fail `nix flake check` at eval) ---
  #
  # Linux only: the actions name absolute store paths here (the bare
  # fallbacks are pinned in `_darwinCapturePins`). The profile turns
  # the slot on; the portal backends, PipeWire and the tools arrive
  # with it.
  _capturePins = lib.optionals isLinux [
    # Home-manager: the whole slot on (the tools beside the
    # profile's own, the per-desktop chooser file xdpw reads first)...
    (
      assert allAssertionsHold hmCapture.config;
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.capture.enable;
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.capture.chooser == "fuzzel";
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.capture.maxFps == 30;
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.capture.outputName == null;
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.theme.targets.capture.enable;
      true
    )
    # ...running stock nixpkgs tools (the same derivations both sides
    # install, so a divergent package fails here)...
    (
      assert hmCapture.config.programs.scoot.desktop.capture.grimPackage.drvPath == pkgs.grim.drvPath;
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.capture.slurpPackage.drvPath == pkgs.slurp.drvPath;
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.capture.menuPackage.drvPath == pkgs.fuzzel.drvPath;
      true
    )
    (
      assert
        hmCapture.config.programs.scoot.desktop.capture.menuPackage.drvPath
        == hmCapture.config.programs.scoot.desktop.clipboard.menuPackage.drvPath;
      true
    )
    (
      assert
        hmCapture.config.programs.scoot.desktop.capture.wlClipboardPackage.drvPath
        == pkgs.wl-clipboard.drvPath;
      true
    )
    # ...installed beside the profile's own, and the chooser file
    # written...
    (
      assert lib.any (p: (p.pname or "") == "grim") hmCapture.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "slurp") hmCapture.config.home.packages;
      true
    )
    (
      assert hmCapture.config.xdg.configFile ? "xdg-desktop-portal-wlr/scoot";
      true
    )
    # ...the slot off: no tools of its own (grim, slurp), no chooser
    # file, and no capture binds (fuzzel and wl-clipboard stay: the
    # launcher and clipboard slots are still on)...
    (
      assert allAssertionsHold hmCaptureOff.config;
      true
    )
    (
      assert !(hmCaptureOff.config.xdg.configFile ? "xdg-desktop-portal-wlr/scoot");
      true
    )
    (
      assert !(lib.any (p: (p.pname or "") == "grim") hmCaptureOff.config.home.packages);
      true
    )
    (
      assert !(lib.any (p: (p.pname or "") == "slurp") hmCaptureOff.config.home.packages);
      true
    )
    (
      assert !(hmCaptureOff.config.programs.scoot.settings.binds ? "print");
      true
    )
    (
      assert !(hmCaptureOff.config.programs.scoot.settings.binds ? "shift+print");
      true
    )
    (
      assert !(hmCaptureOff.config.programs.scoot.settings.binds ? "ctrl+print");
      true
    )
    # ...standalone (no profile): the tools and the file, unthemed...
    (
      assert allAssertionsHold hmCaptureStandalone.config;
      true
    )
    (
      assert hmCaptureStandalone.config.xdg.configFile ? "xdg-desktop-portal-wlr/scoot";
      true
    )
    # ...the other choosers selected...
    (
      assert hmCaptureSlurp.config.programs.scoot.desktop.capture.chooser == "slurp";
      true
    )
    (
      assert allAssertionsHold hmCaptureSlurp.config;
      true
    )
    (
      assert hmCaptureNoneNamed.config.programs.scoot.desktop.capture.outputName == "DP-1";
      true
    )
    (
      assert allAssertionsHold hmCaptureNoneNamed.config;
      true
    )
    (
      assert allAssertionsHold hmCaptureNoneAny.config;
      true
    )
    (
      assert allAssertionsHold hmCaptureTargetOff.config;
      true
    )
    # ...and the refusals naming each tool (a null beside `enable`)...
    (
      assert builtins.length (failing hmCaptureNoGrim.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.grimPackage is null" m) (failing hmCaptureNoGrim.config);
      true
    )
    (
      assert builtins.length (failing hmCaptureNoSlurp.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.slurpPackage is null" m) (failing hmCaptureNoSlurp.config);
      true
    )
    (
      assert builtins.length (failing hmCaptureNoMenu.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.menuPackage is null" m) (failing hmCaptureNoMenu.config);
      true
    )
    (
      assert builtins.length (failing hmCaptureNoCopy.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.wlClipboardPackage is null" m) (
        failing hmCaptureNoCopy.config
      );
      true
    )
    # ...a too-old grim (1.4.0 speaks only the wlr screencopy protocol
    # scoot omits on purpose)...
    (
      assert builtins.length (failing hmCaptureOldGrim.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "too old" m) (failing hmCaptureOldGrim.config);
      true
    )
    # ...a negative frame cap, and an empty fixed output...
    (
      assert builtins.length (failing hmCaptureNegFps.config) == 1;
      true
    )
    (
      assert builtins.length (failing hmCaptureEmptyOutput.config) == 1;
      true
    )
    # ...and an unknown chooser, which is an option type error (the
    # `enum`'s own message names the valid values), caught here by
    # `tryEval`.
    (
      assert !hmCaptureChooserBogus.success;
      true
    )
    # NixOS: the profile turns the slot on (the portal service with
    # both backends, the `scoot` backend selection, PipeWire, the
    # tools on PATH, the fallback chooser config)...
    (
      assert allAssertionsHold osCapture.config;
      true
    )
    (
      assert osCapture.config.programs.scoot.desktop.capture.enable;
      true
    )
    (
      assert osCapture.config.xdg.portal.enable;
      true
    )
    (
      assert
        drvs osCapture.config.xdg.portal.extraPortals == drvs [
          pkgs.xdg-desktop-portal-wlr
          pkgs.xdg-desktop-portal-gtk
        ];
      true
    )
    (
      assert
        osCapture.config.xdg.portal.config.scoot == {
          default = [ "gtk" ];
          "org.freedesktop.impl.portal.Screenshot" = [ "wlr" ];
          "org.freedesktop.impl.portal.ScreenCast" = [ "wlr" ];
        };
      true
    )
    (
      assert osCapture.config.services.pipewire.enable;
      true
    )
    (
      assert lib.any (
        p: (p.pname or "") == "xdg-desktop-portal-wlr"
      ) osCapture.config.environment.systemPackages;
      true
    )
    (
      assert lib.any (
        p: (p.pname or "") == "xdg-desktop-portal-gtk"
      ) osCapture.config.environment.systemPackages;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "grim") osCapture.config.environment.systemPackages;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "slurp") osCapture.config.environment.systemPackages;
      true
    )
    (
      assert osCapture.config.environment.etc ? "xdg/xdg-desktop-portal-wlr/config";
      true
    )
    # ...the system fallback chooser behind it (unthemed: the look is
    # per-user, so the themed flags live only in the per-desktop
    # file above -- and a bare store path, for the same inih line
    # limit)...
    (
      assert lib.hasInfix "chooser_cmd="
        osCapture.config.environment.etc."xdg/xdg-desktop-portal-wlr/config".text;
      true
    )
    (
      assert lib.hasInfix "scoot-screencast-chooser"
        osCapture.config.environment.etc."xdg/xdg-desktop-portal-wlr/config".text;
      true
    )
    (
      assert lib.hasInfix "chooser_type=dmenu"
        osCapture.config.environment.etc."xdg/xdg-desktop-portal-wlr/config".text;
      true
    )
    (
      assert lib.hasInfix "max_fps=30"
        osCapture.config.environment.etc."xdg/xdg-desktop-portal-wlr/config".text;
      true
    )
    # ...the slot off: no portal service and no backends on PATH --
    # but PipeWire stays: the audio slot (on with the profile too)
    # defaults the same switch, which merges rather than conflicts...
    (
      assert allAssertionsHold osCaptureOff.config;
      true
    )
    (
      assert !osCaptureOff.config.xdg.portal.enable;
      true
    )
    (
      assert osCaptureOff.config.services.pipewire.enable;
      true
    )
    # ...and with the audio slot off too, PipeWire stops (the switch
    # follows its last user)...
    (
      assert allAssertionsHold osCaptureAudioOff.config;
      true
    )
    (
      assert !osCaptureAudioOff.config.services.pipewire.enable;
      true
    )
    (
      assert
        !(lib.any (
          p: (p.pname or "") == "xdg-desktop-portal-wlr"
        ) osCaptureOff.config.environment.systemPackages);
      true
    )
    # ...standalone (no profile): the portal service without the
    # session entry...
    (
      assert allAssertionsHold osCaptureStandalone.config;
      true
    )
    (
      assert osCaptureStandalone.config.xdg.portal.enable;
      true
    )
    # ...and each refusal naming its tool on this side as well...
    (
      assert builtins.length (failing osCaptureNoWlr.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.portalWlrPackage is null" m) (
        failing osCaptureNoWlr.config
      );
      true
    )
    (
      assert builtins.length (failing osCaptureNoGtk.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.portalGtkPackage is null" m) (
        failing osCaptureNoGtk.config
      );
      true
    )
    (
      assert builtins.length (failing osCaptureNoGrim.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.grimPackage is null" m) (failing osCaptureNoGrim.config);
      true
    )
    (
      assert builtins.length (failing osCaptureNoSlurp.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.slurpPackage is null" m) (failing osCaptureNoSlurp.config);
      true
    )
    (
      assert builtins.length (failing osCaptureNoMenu.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.menuPackage is null" m) (failing osCaptureNoMenu.config);
      true
    )
    # ...a 0.8.3 backend (stalls recordings) and a too-old grim...
    (
      assert builtins.length (failing osCaptureOldWlr.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "too old" m) (failing osCaptureOldWlr.config);
      true
    )
    (
      assert builtins.length (failing osCaptureOldGrim.config) == 1;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "too old" m) (failing osCaptureOldGrim.config);
      true
    )
  ];

  # --- audio slot (fail `nix flake check` at eval) ---
  #
  # Linux only: the actions name absolute store paths here (the bare
  # fallbacks are pinned in `_darwinAudioPins`). The profile turns the
  # slot on; PipeWire and the OSD arrive with it.
  _audioPins = lib.optionals isLinux [
    # Home-manager: the whole slot on (the OSD and its unit, the
    # control scripts, the sink helper), the OSD themed by the look.
    (
      assert allAssertionsHold hmAudio.config;
      true
    )
    (
      assert hmAudio.config.programs.scoot.desktop.audio.enable;
      true
    )
    (
      assert hmAudio.config.programs.scoot.desktop.audio.daemon == "wob";
      true
    )
    (
      assert hmAudio.config.programs.scoot.desktop.audio.osd.timeoutMs == 1500;
      true
    )
    (
      assert hmAudio.config.programs.scoot.desktop.theme.targets.osd.enable;
      true
    )
    (
      assert !hmAudio.config.programs.scoot.desktop.theme.enable;
      true
    )
    (
      assert hmAudio.config.xdg.configFile ? "wob/wob.ini";
      true
    )
    # ...the OSD unit in the session scope (never the shared target),
    # running the OSD script's daemon...
    (
      assert hmAudio.config.systemd.user.services.scoot-osd.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert hmAudio.config.systemd.user.services.scoot-osd.Unit.After == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmAudio.config.systemd.user.services.scoot-osd.Install.WantedBy == [ "scoot-session.target" ];
      true
    )
    (
      assert lib.hasSuffix "/bin/scoot-osd daemon"
        hmAudio.config.systemd.user.services.scoot-osd.Service.ExecStart;
      true
    )
    # ...the volume, brightness and mic-mute binds through the scripts
    # (control plus OSD -- the keymap's own exact match in `_keysPins`
    # pins every routed action; here the off shape)...
    (
      assert allAssertionsHold hmAudioOff.config;
      true
    )
    (
      assert
        hmAudioOff.config.programs.scoot.settings.binds."XF86AudioRaiseVolume" == {
          action = "spawn ${lib.getExe' pkgs.wireplumber "wpctl"} set-volume @DEFAULT_AUDIO_SINK@ 5%+";
          repeat = true;
          allow_when_locked = true;
        };
      true
    )
    (
      assert
        hmAudioOff.config.programs.scoot.settings.binds."XF86MonBrightnessUp" == {
          action = "spawn ${lib.getExe pkgs.brightnessctl} -e set +5%";
          repeat = true;
          allow_when_locked = true;
        };
      true
    )
    # ...with the slot off: no unit, no config, no scripts...
    (
      assert !(hmAudioOff.config.systemd.user.services ? scoot-osd);
      true
    )
    (
      assert !(hmAudioOff.config.xdg.configFile ? "wob/wob.ini");
      true
    )
    # ...without a look: the slot runs unthemed (wob's own colors,
    # the muted style still defined -- pinned by content below)...
    (
      assert allAssertionsHold hmAudioNoLook.config;
      true
    )
    # ...standalone (no profile): the slot runs, the unit scoped...
    (
      assert allAssertionsHold hmAudioStandalone.config;
      true
    )
    (
      assert
        hmAudioStandalone.config.systemd.user.services.scoot-osd.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    # ...retimed and opted out...
    (
      assert hmAudioTimeout.config.programs.scoot.desktop.audio.osd.timeoutMs == 2500;
      true
    )
    (
      assert !hmAudioTargetOff.config.programs.scoot.desktop.theme.targets.osd.enable;
      true
    )
    # Refusals: the slot with no OSD to run it...
    (
      assert builtins.length (failing hmAudioNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "audio.osd.package is null" (builtins.head (failing hmAudioNoPkg.config));
      true
    )
    # ...with no dump tool for the sink helper...
    (
      assert builtins.length (failing hmAudioNoDump.config) == 1;
      true
    )
    (
      assert lib.hasInfix "audio.dumpPackage is null" (builtins.head (failing hmAudioNoDump.config));
      true
    )
    # ...with a negative hide timeout...
    (
      assert builtins.length (failing hmAudioNegTimeout.config) == 1;
      true
    )
    (
      assert lib.hasInfix "audio.osd.timeoutMs" (builtins.head (failing hmAudioNegTimeout.config));
      true
    )
    # ...and an unknown daemon, which is an option type error (the
    # `enum`'s own message names the valid value).
    (
      assert !hmAudioDaemonBogus.success;
      true
    )
    # NixOS: the profile turns the slot on (PipeWire running, the OSD
    # installed)...
    (
      assert allAssertionsHold osAudio.config;
      true
    )
    (
      assert osAudio.config.programs.scoot.desktop.audio.enable;
      true
    )
    (
      assert osAudio.config.services.pipewire.enable;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "wob") osAudio.config.environment.systemPackages;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "pipewire") osAudio.config.environment.systemPackages;
      true
    )
    # ...the slot off: neither installed...
    (
      assert allAssertionsHold osAudioOff.config;
      true
    )
    (
      assert !(lib.any (p: (p.pname or "") == "wob") osAudioOff.config.environment.systemPackages);
      true
    )
    # ...standalone (no profile): the slot runs...
    (
      assert allAssertionsHold osAudioStandalone.config;
      true
    )
    # ...and the shared `osd.timeoutMs` exists on the NixOS side too (the
    # nested `osd` is merged, not replaced by the package option)...
    (
      assert allAssertionsHold osAudioTimeout.config;
      assert osAudioTimeout.config.programs.scoot.desktop.audio.osd.timeoutMs == 3000;
      true
    )
    # Refusals: the slot with no OSD or dump tool to install (each
    # pinned by message)...
    (
      assert builtins.length (failing osAudioNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "audio.osd.package is null" (builtins.head (failing osAudioNoPkg.config));
      true
    )
    (
      assert builtins.length (failing osAudioNoDump.config) == 1;
      true
    )
    (
      assert lib.hasInfix "audio.dumpPackage is null" (builtins.head (failing osAudioNoDump.config));
      true
    )
  ];

  _darwinAudioPins = lib.optionals (!isLinux) [
    # Home-manager: every tool null, nothing installed for the slot...
    (
      assert hmAudio.config.programs.scoot.desktop.audio.osd.package == null;
      true
    )
    (
      assert hmAudio.config.programs.scoot.desktop.audio.dumpPackage == null;
      true
    )
    # ...and the slot's own assertions refusing loudly, naming each
    # tool (this slot's two)...
    (
      assert lib.any (m: lib.hasInfix "audio.osd.package is null" m) (failing hmAudio.config);
      true
    )
    (
      assert lib.any (m: lib.hasInfix "audio.dumpPackage is null" m) (failing hmAudio.config);
      true
    )
    # Standalone (no profile): just the slot's own two.
    (
      assert builtins.length (failing hmAudioStandalone.config) == 2;
      true
    )
    # NixOS: the same nulls (no OSD installed for the slot)...
    (
      assert osAudio.config.programs.scoot.desktop.audio.osd.package == null;
      true
    )
    (
      assert osAudio.config.programs.scoot.desktop.audio.dumpPackage == null;
      true
    )
    # ...refused loudly there too.
    (
      assert builtins.length (failing osAudioStandalone.config) == 2;
      true
    )
  ];

  # --- capture slot off Linux (fail `nix flake check` at eval) ---
  #
  # The tools above are Linux-only: off Linux each package defaults to
  # null, which the slot's own assertions refuse loudly instead of
  # installing nothing silently. Empty off Linux (the Linux check
  # above is where the slot is pinned).
  _darwinCapturePins = lib.optionals (!isLinux) [
    # Home-manager: every tool null, nothing installed for the slot...
    (
      assert hmCapture.config.programs.scoot.desktop.capture.grimPackage == null;
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.capture.slurpPackage == null;
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.capture.menuPackage == null;
      true
    )
    (
      assert hmCapture.config.programs.scoot.desktop.capture.wlClipboardPackage == null;
      true
    )
    # ...and the slot's own assertions refusing loudly, naming each
    # tool (the idle policy's five plus the daemon's one plus the
    # launcher's one plus the clipboard slot's three plus this slot's
    # four plus the audio slot's two plus the night light's one:
    # the profile is on in this evaluation).
    (
      assert builtins.length (failing hmCapture.config) == 17;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.grimPackage is null" m) (failing hmCapture.config);
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.slurpPackage is null" m) (failing hmCapture.config);
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.menuPackage is null" m) (failing hmCapture.config);
      true
    )
    (
      assert lib.any (m: lib.hasInfix "capture.wlClipboardPackage is null" m) (failing hmCapture.config);
      true
    )
    # Standalone (no profile): just the slot's own four.
    (
      assert builtins.length (failing hmCaptureStandalone.config) == 4;
      true
    )
    # NixOS: the same nulls (no backends installed for the slot)...
    (
      assert osCapture.config.programs.scoot.desktop.capture.portalWlrPackage == null;
      true
    )
    (
      assert osCapture.config.programs.scoot.desktop.capture.portalGtkPackage == null;
      true
    )
    # ...refused loudly there too (the idle policy's five plus the
    # daemon's one plus the launcher's one plus the clipboard slot's
    # three plus this slot's five -- gtk beside wlr -- plus the
    # audio slot's two plus the night light's one).
    (
      assert builtins.length (failing osCapture.config) == 18;
      true
    )
  ];

  # --- clipboard slot off Linux (fail `nix flake check` at eval) ---
  #
  # The tools above are Linux-only: off Linux each package defaults to
  # null, which the slot's own assertions refuse loudly instead of
  # installing nothing silently. Empty off Linux (the Linux check above
  # is where the slot is pinned).
  _darwinClipPins = lib.optionals (!isLinux) [
    # Home-manager: every tool null, nothing installed for the slot...
    (
      assert hmClip.config.programs.scoot.desktop.clipboard.managerPackage == null;
      true
    )
    (
      assert hmClip.config.programs.scoot.desktop.clipboard.wlClipboardPackage == null;
      true
    )
    (
      assert hmClip.config.programs.scoot.desktop.clipboard.menuPackage == null;
      true
    )
    # ...and the slot's own assertions refusing loudly, naming each
    # switch (the idle policy's five plus the daemon's one plus the
    # launcher's one plus the slot's three plus the capture slot's
    # four plus the audio slot's two plus the night light's one:
    # the profile is on in this evaluation).
    (
      assert builtins.length (failing hmClip.config) == 17;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "clipboard.managerPackage is null" m) (failing hmClip.config);
      true
    )
    (
      assert lib.any (m: lib.hasInfix "clipboard.wlClipboardPackage is null" m) (failing hmClip.config);
      true
    )
    (
      assert lib.any (m: lib.hasInfix "clipboard.menuPackage is null" m) (failing hmClip.config);
      true
    )
    # Standalone (no profile): just the slot's own three.
    (
      assert builtins.length (failing hmClipStandalone.config) == 3;
      true
    )
    # NixOS: the same nulls (no tools installed for the slot)...
    (
      assert osClip.config.programs.scoot.desktop.clipboard.managerPackage == null;
      true
    )
    # ...refused loudly there too (one more than before: the
    # launcher's null joins the count on this side as well -- plus
    # the audio slot's two plus the night light's one beside it).
    (
      assert builtins.length (failing osClip.config) == 18;
      true
    )
  ];

  # --- notification daemon off Linux (fail `nix flake check` at eval) ---
  #
  # mako is Linux-only: off Linux its package defaults to null, which
  # the daemon's own assertion refuses loudly instead of installing
  # nothing silently. Empty off Linux (the Linux check above is where
  # the daemon is pinned).
  _darwinNotifPins = lib.optionals (!isLinux) [
    # Home-manager: null, and the daemon's assertion refusing loudly
    # (the idle policy's five plus the daemon's one plus the launcher's
    # one plus the clipboard slot's three plus the capture slot's
    # four plus the audio slot's two plus the night light's one:
    # the profile is on in this evaluation, so its slot is open).
    (
      assert hmNotif.config.programs.scoot.desktop.notifications.package == null;
      true
    )
    (
      assert builtins.length (failing hmNotif.config) == 17;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "notifications.package is null" m) (failing hmNotif.config);
      true
    )
    # NixOS: the same null (no daemon installed)...
    (
      assert osNotif.config.programs.scoot.desktop.notifications.package == null;
      true
    )
    # ...refused loudly there too (the idle policy's five plus the
    # daemon's one plus the launcher's one plus the clipboard slot's
    # three plus the capture slot's five -- gtk beside wlr -- plus the
    # audio slot's two plus the night light's one: the
    # profile is on in this evaluation, so its slot is open).
    (
      assert builtins.length (failing osNotif.config) == 18;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "notifications.package is null" m) (failing osNotif.config);
      true
    )
    # ...while the bar feed stays unwritten (no daemon, no toggle).
    (
      assert (hmDeskBarNotif.config.programs.scootbar.settings.push or { }) == { };
      true
    )
  ];

  # --- night light (`programs.scoot.desktop.nightlight`) pins (fail
  # `nix flake check` at eval) ---
  #
  # Linux only: the unit names absolute store paths here (off Linux the
  # tool is null, refused loudly -- pinned in
  # `_darwinNightlightPins`). The profile turns the slot on; the manual
  # schedule needs neither location nor network, and the gammastep
  # daemon covers location-based sunrise/sunset.
  _nightlightPins = lib.optionals isLinux [
    # Home-manager: the whole slot on with the profile (the tool on
    # PATH, the user unit bound to the session scope)...
    (
      assert allAssertionsHold hmNight.config;
      true
    )
    (
      assert hmNight.config.programs.scoot.desktop.nightlight.enable;
      true
    )
    (
      assert hmNight.config.programs.scoot.desktop.nightlight.daemon == "wlsunset";
      true
    )
    (
      assert hmNight.config.programs.scoot.desktop.nightlight.package.drvPath == pkgs.wlsunset.drvPath;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "wlsunset") hmNight.config.home.packages;
      true
    )
    (
      assert
        hmNight.config.systemd.user.services.scoot-nightlight.Install.WantedBy
        == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmNight.config.systemd.user.services.scoot-nightlight.Unit.PartOf == [ "scoot-session.target" ];
      true
    )
    (
      assert
        hmNight.config.systemd.user.services.scoot-nightlight.Unit.After == [ "scoot-session.target" ];
      true
    )
    # ...warming to the look's own night temperature (music-desk keeps
    # most blue at 4000)...
    (
      assert hmNight.config.programs.scoot.desktop.nightlight.nightTemp == 4000;
      true
    )
    # ...each remaining look warming to its own (vinyl-sunset warmest,
    # moonrise and radial-burst between)...
    (
      assert
        (evalHome {
          enable = true;
          desktop.enable = true;
          desktop.look = "vinyl-sunset";
        }).config.programs.scoot.desktop.nightlight.nightTemp == 3200;
      true
    )
    (
      assert
        (evalHome {
          enable = true;
          desktop.enable = true;
          desktop.look = "moonrise";
        }).config.programs.scoot.desktop.nightlight.nightTemp == 3400;
      true
    )
    (
      assert
        (evalHome {
          enable = true;
          desktop.enable = true;
          desktop.look = "radial-burst";
        }).config.programs.scoot.desktop.nightlight.nightTemp == 3500;
      true
    )
    # ...a user value winning per key over the look's...
    (
      assert hmNightUserWins.config.programs.scoot.desktop.nightlight.nightTemp == 3000;
      true
    )
    (
      assert allAssertionsHold hmNightUserWins.config;
      true
    )
    # ...opted out of look theming (the plain 3500 stands while the
    # rest follows the look)...
    (
      assert hmNightTargetOff.config.programs.scoot.desktop.nightlight.nightTemp == 3500;
      true
    )
    (
      assert allAssertionsHold hmNightTargetOff.config;
      true
    )
    # ...and without a look the plain default stands.
    (
      assert hmNightNoLook.config.programs.scoot.desktop.nightlight.nightTemp == 3500;
      true
    )
    (
      assert allAssertionsHold hmNightNoLook.config;
      true
    )
    # The unit runs the manual schedule by default (day 6500, night
    # from the look, 07:00/19:00 over 15 min, gamma neutral)...
    (
      assert lib.hasInfix "wlsunset -T 6500 -t 4000 -S 07:00 -s 19:00 -d 900 -g 1.000000"
        hmNight.config.systemd.user.services.scoot-nightlight.Service.ExecStart;
      true
    )
    # ...location mode instead with the pair set (the manual times and
    # the duration unread: the sun computes the boundaries)...
    (
      assert allAssertionsHold hmNightLocated.config;
      true
    )
    (
      assert lib.hasInfix "wlsunset -T 6500 -t 3500 -l 37.330000 -L -121.890000 -g 1.000000"
        hmNightLocated.config.systemd.user.services.scoot-nightlight.Service.ExecStart;
      true
    )
    (
      assert
        !(lib.hasInfix "-S" hmNightLocated.config.systemd.user.services.scoot-nightlight.Service.ExecStart);
      true
    )
    # ...and the gammastep daemon in its own spelling (Wayland method,
    # day:night, location, per-channel gamma).
    (
      assert allAssertionsHold hmNightGamma.config;
      true
    )
    (
      assert
        hmNightGamma.config.programs.scoot.desktop.nightlight.package.drvPath == pkgs.gammastep.drvPath;
      true
    )
    (
      assert lib.hasInfix
        "gammastep -m wayland -t 6500:3500 -l 37.330000:-121.890000 -g 1.000000:1.000000:1.000000"
        hmNightGamma.config.systemd.user.services.scoot-nightlight.Service.ExecStart;
      true
    )
    # ...the slot off: no tool, no unit (the profile's own stay)...
    (
      assert allAssertionsHold hmNightOff.config;
      true
    )
    (
      assert !(hmNightOff.config.systemd.user.services ? scoot-nightlight);
      true
    )
    (
      assert !(lib.any (p: (p.pname or "") == "wlsunset") hmNightOff.config.home.packages);
      true
    )
    # ...standalone (no profile): the tool and the unit, unthemed, with
    # the session scope installed for it...
    (
      assert allAssertionsHold hmNightStandalone.config;
      true
    )
    (
      assert hmNightStandalone.config.systemd.user.services ? scoot-nightlight;
      true
    )
    (
      assert hmNightStandalone.config.xdg.configFile ? "systemd/user/scoot-session.target";
      true
    )
    # Refusals, each naming its switch: no tool to run...
    (
      assert builtins.length (failing hmNightNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "nightlight.package is null" (builtins.head (failing hmNightNoPkg.config));
      true
    )
    # ...a day outside 1000..10000, a night outside it, and a night
    # bluer than the day...
    (
      assert builtins.length (failing hmNightBadDay.config) == 1;
      true
    )
    (
      assert lib.hasInfix "nightlight.dayTemp" (builtins.head (failing hmNightBadDay.config));
      true
    )
    (
      assert builtins.length (failing hmNightBadNight.config) == 1;
      true
    )
    (
      assert lib.hasInfix "nightlight.nightTemp" (builtins.head (failing hmNightBadNight.config));
      true
    )
    (
      assert builtins.length (failing hmNightNightAboveDay.config) == 1;
      true
    )
    (
      assert lib.hasInfix "warmer" (builtins.head (failing hmNightNightAboveDay.config));
      true
    )
    # ...a sunrise/sunset outside 24-hour `HH:MM`...
    (
      assert builtins.length (failing hmNightBadRise.config) == 1;
      true
    )
    (
      assert lib.hasInfix "nightlight.sunrise" (builtins.head (failing hmNightBadRise.config));
      true
    )
    (
      assert builtins.length (failing hmNightBadSet.config) == 1;
      true
    )
    (
      assert lib.hasInfix "nightlight.sunset" (builtins.head (failing hmNightBadSet.config));
      true
    )
    # ...a transition outside 0..7200, and a multiplier outside
    # 0.1..10...
    (
      assert builtins.length (failing hmNightBadDuration.config) == 1;
      true
    )
    (
      assert lib.hasInfix "nightlight.duration" (builtins.head (failing hmNightBadDuration.config));
      true
    )
    (
      assert builtins.length (failing hmNightBadGamma.config) == 1;
      true
    )
    (
      assert lib.hasInfix "nightlight.gamma" (builtins.head (failing hmNightBadGamma.config));
      true
    )
    # ...a latitude outside its degrees, and one coordinate without
    # the other...
    (
      assert builtins.length (failing hmNightBadLat.config) == 1;
      true
    )
    (
      assert lib.hasInfix "nightlight.latitude" (builtins.head (failing hmNightBadLat.config));
      true
    )
    (
      assert builtins.length (failing hmNightHalfLoc.config) == 1;
      true
    )
    (
      assert lib.hasInfix "only one of" (builtins.head (failing hmNightHalfLoc.config));
      true
    )
    # ...gammastep with nowhere to stand...
    (
      assert builtins.length (failing hmNightGammaNoLoc.config) == 1;
      true
    )
    (
      assert lib.hasInfix "is \"gammastep\" with" (builtins.head (failing hmNightGammaNoLoc.config));
      true
    )
    # ...and an unknown daemon, an enum type error (verified by hand
    # to name the two valid values).
    (
      assert !hmNightDaemonBogus.success;
      true
    )
    # NixOS: the profile installs the tool system-wide, still additive
    # (no default session)...
    (
      assert allAssertionsHold osNight.config;
      true
    )
    (
      assert osNight.config.programs.scoot.desktop.nightlight.enable;
      true
    )
    (
      assert osNight.config.programs.scoot.desktop.nightlight.package.drvPath == pkgs.wlsunset.drvPath;
      true
    )
    (
      assert lib.any (p: (p.pname or "") == "wlsunset") osNight.config.environment.systemPackages;
      true
    )
    # ...the slot off: the profile's own packages only...
    (
      assert allAssertionsHold osNightOff.config;
      true
    )
    (
      assert !(lib.any (p: (p.pname or "") == "wlsunset") osNightOff.config.environment.systemPackages);
      true
    )
    # ...standalone (no profile): the tool without the session entry...
    (
      assert allAssertionsHold osNightStandalone.config;
      true
    )
    (
      assert builtins.length osNightStandalone.config.services.displayManager.sessionPackages == 0;
      true
    )
    (
      assert lib.any (
        p: (p.pname or "") == "wlsunset"
      ) osNightStandalone.config.environment.systemPackages;
      true
    )
    # ...and the refusal names the switch on this side as well.
    (
      assert builtins.length (failing osNightNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "nightlight.package is null" (builtins.head (failing osNightNoPkg.config));
      true
    )
  ];

  # --- night light off Linux (fail `nix flake check` at eval) ---
  #
  # The tool is Linux-only like the idle policy's: off Linux its
  # package defaults to null, which the slot's own assertion refuses
  # loudly instead of installing nothing silently. The schedule values
  # are plain data, so they still land. (The Linux check above is where
  # the slot is pinned.)
  _darwinNightlightPins = lib.optionals (!isLinux) [
    # Home-manager: null, the daemon following it (wlsunset without a
    # package to name)...
    (
      assert hmNight.config.programs.scoot.desktop.nightlight.package == null;
      true
    )
    # ...refused loudly beside the profile's other sixteen (the idle
    # policy's five, the daemon's one, the launcher's one, the
    # clipboard slot's three, the capture slot's four, the audio
    # slot's two, the night light's one).
    (
      assert builtins.length (failing hmNight.config) == 17;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "nightlight.package is null" m) (failing hmNight.config);
      true
    )
    # NixOS: the same null (no tool installed)...
    (
      assert osNight.config.programs.scoot.desktop.nightlight.package == null;
      true
    )
    # ...refused loudly there too (the idle policy's five plus the
    # daemon's one plus the launcher's one plus the clipboard slot's
    # three plus the capture slot's five -- gtk beside wlr -- plus the
    # audio slot's two plus the night light's one).
    (
      assert builtins.length (failing osNight.config) == 18;
      true
    )
  ];

  # --- shared keymap (fail `nix flake check` at eval) ---
  #
  # Linux only: the actions name absolute store paths here (the bare
  # fallbacks are pinned in `_darwinKeysPins`). Every default bind is
  # pinned by combo and action below; an override and a removal prove
  # the two user paths.
  _keysPins = lib.optionals isLinux [
    # Home-manager: the profile turns the keymap on...
    (
      assert allAssertionsHold hmKeys.config;
      true
    )
    (
      assert hmKeys.config.programs.scoot.desktop.keys.enable;
      true
    )
    # ...rendering exactly the twenty-one binds beside the profile
    # (the twelve keymap-owned binds plus the three notification
    # binds, the clipboard picker, the two launcher binds and the
    # three capture binds -- the daemon, the clipboard slot, the
    # launcher slot and the capture slot are on with the profile, so
    # their slots are open; every other future slot is off: its binds
    # stay out; the volume, brightness and mic-mute binds run through
    # the audio slot's scripts, which is on with the profile too)...
    (
      assert
        hmKeys.config.programs.scoot.settings.binds == {
          "XF86MonBrightnessUp" = {
            action = "spawn ${slotScriptBin hmKeys "scoot-brightness"} up";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86MonBrightnessDown" = {
            action = "spawn ${slotScriptBin hmKeys "scoot-brightness"} down";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioRaiseVolume" = {
            action = "spawn ${slotScriptBin hmKeys "scoot-volume"} sink-up";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioLowerVolume" = {
            action = "spawn ${slotScriptBin hmKeys "scoot-volume"} sink-down";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioMute" = {
            action = "spawn ${slotScriptBin hmKeys "scoot-volume"} sink-mute";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioMicMute" = {
            action = "spawn ${slotScriptBin hmKeys "scoot-volume"} mic-mute";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioPlay" = {
            action = "spawn ${lib.getExe pkgs.playerctl} play-pause";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioPause" = {
            action = "spawn ${lib.getExe pkgs.playerctl} pause";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioStop" = {
            action = "spawn ${lib.getExe pkgs.playerctl} stop";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioNext" = {
            action = "spawn ${lib.getExe pkgs.playerctl} next";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioPrev" = {
            action = "spawn ${lib.getExe pkgs.playerctl} previous";
            repeat = true;
            allow_when_locked = true;
          };
          "super+escape" = "spawn ${lib.getExe' pkgs.systemd "loginctl"} lock-session";
          "super+d" = "spawn ${slotScriptBin hmKeys "scoot-launcher"}";
          "ctrl+alt+space" = "spawn ${slotScriptBin hmKeys "scoot-launcher"} --list-executables-in-path";
          "super+v" = "spawn ${slotScriptBin hmKeys "scoot-clipboard-pick"}";
          "super+n" = "spawn ${leanMako}/bin/makoctl dismiss";
          "super+shift+n" = "spawn ${leanMako}/bin/makoctl mode -t do-not-disturb";
          "super+ctrl+n" = "spawn ${leanMako}/bin/makoctl restore";
          "print" = "spawn ${slotScriptBin hmKeys "scoot-capture-output"}";
          "shift+print" = "spawn ${slotScriptBin hmKeys "scoot-capture-region"}";
          "ctrl+print" = "spawn ${slotScriptBin hmKeys "scoot-capture-clipboard"}";
        };
      true
    )
    # ...beside the profile's and the policy's packages (scoot, the
    # five idle tools, mako, the clipboard slot's three, the launcher
    # package, the capture slot's four tools, the audio slot's OSD and
    # its four scripts, the five slot scripts
    # and the keymap's three)...
    (
      assert
        sorted hmKeys.config.home.packages == sorted [
          fakePkg
          pkgs.swayidle
          pkgs.brightnessctl
          pkgs.wlopm
          pkgs.swaylock
          pkgs.sway-audio-idle-inhibit
          leanMako
          leanClip
          pkgs.wl-clipboard
          pkgs.fuzzel
          pkgs.fuzzel
          (slotScriptDrv hmKeys "scoot-clipboard-pick")
          (slotScriptDrv hmKeys "scoot-launcher")
          pkgs.grim
          pkgs.slurp
          pkgs.fuzzel
          pkgs.wl-clipboard
          (slotScriptDrv hmKeys "scoot-capture-output")
          (slotScriptDrv hmKeys "scoot-capture-region")
          (slotScriptDrv hmKeys "scoot-capture-clipboard")
          pkgs.wob
          (slotScriptDrv hmKeys "scoot-osd")
          (slotScriptDrv hmKeys "scoot-volume")
          (slotScriptDrv hmKeys "scoot-brightness")
          (slotScriptDrv hmKeys "scoot-audio-sink")
          pkgs.wlsunset
          pkgs.brightnessctl
          pkgs.wireplumber
          pkgs.playerctl
        ];
      true
    )
    # ...every other future slot on: all twenty-one binds (the
    # launcher, clipboard and capture binds through the keymap's own
    # scripts, the notification binds through mako's absolute path)...
    (
      assert allAssertionsHold hmKeysSlots.config;
      true
    )
    (
      assert builtins.length (builtins.attrNames hmKeysSlots.config.programs.scoot.settings.binds) == 21;
      true
    )
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."super+d"
        == "spawn ${slotScriptBin hmKeysSlots "scoot-launcher"}";
      true
    )
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."ctrl+alt+space"
        == "spawn ${slotScriptBin hmKeysSlots "scoot-launcher"} --list-executables-in-path";
      true
    )
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."super+v"
        == "spawn ${slotScriptBin hmKeysSlots "scoot-clipboard-pick"}";
      true
    )
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."super+n"
        == "spawn ${leanMako}/bin/makoctl dismiss";
      true
    )
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."super+shift+n"
        == "spawn ${leanMako}/bin/makoctl mode -t do-not-disturb";
      true
    )
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."super+ctrl+n"
        == "spawn ${leanMako}/bin/makoctl restore";
      true
    )
    # ...and the DND key runs the bar toggle's own command (both read
    # `notifications.package`, pinned here so a later change to either
    # side fails loudly)...
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."super+shift+n"
        == "spawn ${lib.concatStringsSep " " hmDeskBarNotif.config.programs.scootbar.settings.push.notifications.on-click.exec}";
      true
    )
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."print"
        == "spawn ${slotScriptBin hmKeysSlots "scoot-capture-output"}";
      true
    )
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."shift+print"
        == "spawn ${slotScriptBin hmKeysSlots "scoot-capture-region"}";
      true
    )
    (
      assert
        hmKeysSlots.config.programs.scoot.settings.binds."ctrl+print"
        == "spawn ${slotScriptBin hmKeysSlots "scoot-capture-clipboard"}";
      true
    )
    # ...the four slot scripts installed beside the keymap's tools...
    (
      assert lib.any (p: (p.name or "") == "scoot-clipboard-pick") hmKeysSlots.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.name or "") == "scoot-capture-output") hmKeysSlots.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.name or "") == "scoot-capture-region") hmKeysSlots.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.name or "") == "scoot-capture-clipboard") hmKeysSlots.config.home.packages;
      true
    )
    # ...and the audio slot's four beside them (the OSD and its three
    # control scripts, on with the profile)...
    (
      assert lib.any (p: (p.name or "") == "scoot-osd") hmKeysSlots.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.name or "") == "scoot-volume") hmKeysSlots.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.name or "") == "scoot-brightness") hmKeysSlots.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.name or "") == "scoot-audio-sink") hmKeysSlots.config.home.packages;
      true
    )
    # ...one bind removed: its combo unbound, the other nineteen
    # still there (twenty-one with the daemon, the clipboard slot,
    # the launcher slot and the capture slot on, minus two)...
    (
      assert allAssertionsHold hmKeysOmit.config;
      true
    )
    (
      assert !(hmKeysOmit.config.programs.scoot.settings.binds ? "XF86AudioRaiseVolume");
      true
    )
    (
      assert !(hmKeysOmit.config.programs.scoot.settings.binds ? "super+escape");
      true
    )
    (
      assert builtins.length (builtins.attrNames hmKeysOmit.config.programs.scoot.settings.binds) == 19;
      true
    )
    # ...one bind overridden: the user's own `[binds]` entry wins...
    (
      assert
        hmKeysOverride.config.programs.scoot.settings.binds."XF86AudioRaiseVolume" == "spawn sh -c true";
      true
    )
    (
      assert
        builtins.length (builtins.attrNames hmKeysOverride.config.programs.scoot.settings.binds) == 21;
      true
    )
    # ...a slot bind overridden the same way (the slot's command
    # loses to the user's)...
    (
      assert hmKeysSlotOverride.config.programs.scoot.settings.binds."super+d" == "spawn foot";
      true
    )
    # ...and keys-only (policy and locker off): an empty lock action
    # still fails -- it renders into `super+escape`...
    (
      assert builtins.length (failing hmKeysLockCmdEmpty.config) == 1;
      true
    )
    (
      assert lib.hasInfix "idle.lock.command" (builtins.head (failing hmKeysLockCmdEmpty.config));
      true
    )
    # ...the whole keymap off: no `[binds]` from it (no other eval
    # sets binds here, so the table is absent entirely) and none of
    # the keymap's own slot scripts either (those install beside the
    # binds), so the idle policy's five, mako, the launcher, the
    # clipboard slot's tools, the capture slot's tools, the audio
    # slot's OSD and scripts and the night light's tool beside scoot
    # only...
    (
      assert allAssertionsHold hmKeysOff.config;
      true
    )
    (
      assert !(hmKeysOff.config.programs.scoot.settings ? binds);
      true
    )
    (
      assert
        sorted hmKeysOff.config.home.packages == sorted [
          fakePkg
          pkgs.swayidle
          pkgs.brightnessctl
          pkgs.wlopm
          pkgs.swaylock
          pkgs.sway-audio-idle-inhibit
          leanMako
          pkgs.fuzzel
          leanClip
          pkgs.wl-clipboard
          pkgs.fuzzel
          pkgs.grim
          pkgs.slurp
          pkgs.fuzzel
          pkgs.wl-clipboard
          pkgs.wob
          (slotScriptDrv hmKeysOff "scoot-osd")
          (slotScriptDrv hmKeysOff "scoot-volume")
          (slotScriptDrv hmKeysOff "scoot-brightness")
          (slotScriptDrv hmKeysOff "scoot-audio-sink")
          pkgs.wlsunset
        ];
      true
    )
    # NixOS: the keymap off leaves the profile's other packages
    # only (the five idle tools, mako, the launcher, the clipboard
    # slot's tools, the capture slot's tools, the audio slot's OSD
    # and the night light's tool beside scoot and scootbg).
    (
      assert allAssertionsHold osKeysOff.config;
      true
    )
    (
      assert
        sorted osKeysOff.config.environment.systemPackages == sorted [
          fakePkg
          fakeBg
          pkgs.swayidle
          pkgs.brightnessctl
          pkgs.wlopm
          pkgs.swaylock
          pkgs.sway-audio-idle-inhibit
          leanMako
          pkgs.fuzzel
          leanClip
          pkgs.wl-clipboard
          pkgs.fuzzel
          pkgs.xdg-desktop-portal-wlr
          pkgs.xdg-desktop-portal-gtk
          pkgs.grim
          pkgs.slurp
          pkgs.fuzzel
          pkgs.wob
          pkgs.pipewire
          pkgs.wlsunset
        ];
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
    # steps, the inhibitor, the locker -- plus the notification
    # daemon's one, the launcher's one, the clipboard slot's three,
    # the capture slot's four, the audio slot's two (the OSD and the
    # sink helper's dump tool) and the night light's one,
    # on with the profile; order-insensitive: the daemon's module
    # contributes its refusal first).
    (
      assert builtins.length (failing hmIdle.config) == 17;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "idle.package is null" m) (failing hmIdle.config);
      true
    )
    (
      assert lib.any (m: lib.hasInfix "clipboard.managerPackage is null" m) (failing hmIdle.config);
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
    # ...refused loudly there too (the idle policy's five plus the
    # daemon's one plus the launcher's one plus the clipboard slot's
    # three plus the capture slot's five -- gtk beside wlr -- plus the
    # audio slot's two (the OSD and the sink helper's dump tool) plus
    # the night light's one -- while the
    # docked-lid rule (plain values, no tools) still
    # lands.
    (
      assert builtins.length (failing osIdle.config) == 18;
      true
    )
    (
      assert osIdle.config.services.logind.settings.Login.HandleLidSwitchDocked == "lock";
      true
    )
  ];

  # --- shared keymap off Linux (fail `nix flake check` at eval) ---
  #
  # The keymap's tools are Linux-only like the policy's, but the
  # binds stay benign without them (a missing tool fails quietly at
  # runtime), so off Linux the tools are null, the binds render in
  # their bare form for the Linux box the config deploys to, and --
  # unlike the policy -- the keymap itself refuses nothing (the
  # daemon's null-package refusal beside it is pinned below).
  _darwinKeysPins = lib.optionals (!isLinux) [
    # Home-manager: every keymap tool null, nothing installed for it
    # (scoot itself aside: this eval sets `package`)...
    (
      assert hmKeys.config.programs.scoot.desktop.keys.brightnessPackage == null;
      true
    )
    (
      assert hmKeys.config.programs.scoot.desktop.keys.volumePackage == null;
      true
    )
    (
      assert hmKeys.config.programs.scoot.desktop.keys.mediaPackage == null;
      true
    )
    (
      assert hmKeys.config.home.packages == [ fakePkg ];
      true
    )
    # ...the keymap still on with the profile, its binds in bare
    # form (the lock action bare too: logind is Linux-only -- and the
    # notification binds bare as well: mako is Linux-only, while its
    # slot is open with the profile -- and the clipboard picker, the
    # launcher binds and the capture binds through their own scripts,
    # whose tools are bare there too while their slots are open with
    # the profile)...
    (
      assert hmKeys.config.programs.scoot.desktop.keys.enable;
      true
    )
    # Twenty-one binds: the fifteen above plus the picker, the two
    # launcher binds and the three capture binds, whose store paths
    # are unknowable in the pin (so each is matched by suffix, and
    # the rest byte-equal without them).
    (
      assert
        builtins.removeAttrs hmKeys.config.programs.scoot.settings.binds [
          "super+v"
          "super+d"
          "ctrl+alt+space"
          "print"
          "shift+print"
          "ctrl+print"
        ] == {
          "XF86MonBrightnessUp" = {
            action = "spawn brightnessctl -e set +5%";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86MonBrightnessDown" = {
            action = "spawn brightnessctl -e set 5%-";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioRaiseVolume" = {
            action = "spawn wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioLowerVolume" = {
            action = "spawn wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioMute" = {
            action = "spawn wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioMicMute" = {
            action = "spawn wpctl set-mute @DEFAULT_AUDIO_SOURCE@ toggle";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioPlay" = {
            action = "spawn playerctl play-pause";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioPause" = {
            action = "spawn playerctl pause";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioStop" = {
            action = "spawn playerctl stop";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioNext" = {
            action = "spawn playerctl next";
            repeat = true;
            allow_when_locked = true;
          };
          "XF86AudioPrev" = {
            action = "spawn playerctl previous";
            repeat = true;
            allow_when_locked = true;
          };
          "super+escape" = "spawn loginctl lock-session";
          "super+n" = "spawn makoctl dismiss";
          "super+shift+n" = "spawn makoctl mode -t do-not-disturb";
          "super+ctrl+n" = "spawn makoctl restore";
        };
      true
    )
    (
      assert lib.hasSuffix "/bin/scoot-clipboard-pick"
        hmKeys.config.programs.scoot.settings.binds."super+v";
      true
    )
    (
      assert lib.hasSuffix "/bin/scoot-launcher" hmKeys.config.programs.scoot.settings.binds."super+d";
      true
    )
    (
      assert lib.hasSuffix "/bin/scoot-launcher --list-executables-in-path"
        hmKeys.config.programs.scoot.settings.binds."ctrl+alt+space";
      true
    )
    (
      assert lib.hasSuffix "/bin/scoot-capture-output"
        hmKeys.config.programs.scoot.settings.binds."print";
      true
    )
    (
      assert lib.hasSuffix "/bin/scoot-capture-region"
        hmKeys.config.programs.scoot.settings.binds."shift+print";
      true
    )
    (
      assert lib.hasSuffix "/bin/scoot-capture-clipboard"
        hmKeys.config.programs.scoot.settings.binds."ctrl+print";
      true
    )
    # ...and the picker, both launcher binds and the three capture
    # binds stay plain strings: never repeat, never allowed while
    # locked (unlike the eleven hardware tables above).
    (
      assert lib.isString hmKeys.config.programs.scoot.settings.binds."super+v";
      true
    )
    (
      assert lib.isString hmKeys.config.programs.scoot.settings.binds."super+d";
      true
    )
    (
      assert lib.isString hmKeys.config.programs.scoot.settings.binds."ctrl+alt+space";
      true
    )
    (
      assert lib.isString hmKeys.config.programs.scoot.settings.binds."print";
      true
    )
    (
      assert lib.isString hmKeys.config.programs.scoot.settings.binds."shift+print";
      true
    )
    (
      assert lib.isString hmKeys.config.programs.scoot.settings.binds."ctrl+print";
      true
    )
    (
      assert builtins.length (builtins.attrNames hmKeys.config.programs.scoot.settings.binds) == 21;
      true
    )
    # ...and the keymap refuses nothing itself: the only failing
    # assertions are the idle policy's five plus the daemon's one plus
    # the launcher's one plus the clipboard slot's three plus the
    # capture slot's four plus the audio slot's two plus the night
    # light's one (their
    # packages are null off Linux -- the daemon's pinned in
    # `_darwinNotifPins`, the launcher's in `_darwinLaunchPins`, the
    # clipboard slot's in `_darwinClipPins`, the capture slot's in
    # `_darwinCapturePins`, the audio slot's in `_darwinAudioPins`,
    # the night light's in
    # `_darwinNightlightPins`), so bare tool names stay valid
    # config, just quiet at runtime.
    (
      assert builtins.length (failing hmKeys.config) == 17;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "notifications.package is null" m) (failing hmKeys.config);
      true
    )
    # NixOS: the same nulls, nothing installed for the keymap.
    (
      assert osIdle.config.programs.scoot.desktop.keys.brightnessPackage == null;
      true
    )
    (
      assert osIdle.config.programs.scoot.desktop.keys.volumePackage == null;
      true
    )
    (
      assert osIdle.config.programs.scoot.desktop.keys.mediaPackage == null;
      true
    )
  ];

  # --- power policy (`programs.scoot.desktop.power`) pins (fail
  # `nix flake check` at eval) ---
  #
  # Linux only: the daemon, the logind/UPower policy and the charge
  # service are system-level wirings with no meaning on Darwin (same
  # gating as the idle policy above). Each refusal pin also proves
  # the message names the option (the `hasInfix` half), not just that
  # something fails.
  _powerPins = lib.optionals isLinux [
    # Home-manager: every assertion holds, standalone or with the
    # profile...
    (
      assert allAssertionsHold hmPower.config;
      true
    )
    (
      assert allAssertionsHold hmPowerStandalone.config;
      true
    )
    (
      assert allAssertionsHold hmPowerChargeOff.config;
      true
    )
    (
      assert allAssertionsHold hmPowerChargeBounds.config;
      true
    )
    (
      assert allAssertionsHold hmPowerAuto.config;
      true
    )
    (
      assert allAssertionsHold hmPowerLid.config;
      true
    )
    (
      assert allAssertionsHold hmPowerLowAction.config;
      true
    )
    (
      assert allAssertionsHold hmPowerNoProfiles.config;
      true
    )
    # ...the profiles daemon wanted by default, droppable where no
    # driver honors it...
    (
      assert hmPower.config.programs.scoot.desktop.power.profiles.enable;
      true
    )
    (
      assert !hmPowerNoProfiles.config.programs.scoot.desktop.power.profiles.enable;
      true
    )
    # ...without it no switch script, no daemon package and no bind
    # (twenty-one binds, the keymap without the policy's own)...
    (
      assert
        !(lib.any (p: (p.name or "") == "scoot-power-profile") hmPowerNoProfiles.config.home.packages);
      true
    )
    (
      assert
        !(lib.any (p: (p.pname or "") == "power-profiles-daemon") hmPowerNoProfiles.config.home.packages);
      true
    )
    (
      assert
        builtins.length (builtins.attrNames hmPowerNoProfiles.config.programs.scoot.settings.binds) == 21;
      true
    )
    (
      assert !(hmPowerNoProfiles.config.programs.scoot.settings.binds ? "super+p");
      true
    )
    # ...the charge cap rides with the policy (still individually
    # disable-able)...
    (
      assert hmPower.config.programs.scoot.desktop.power.chargeLimit.enable;
      true
    )
    (
      assert hmPowerStandalone.config.programs.scoot.desktop.power.chargeLimit.enable;
      true
    )
    (
      assert !hmPowerChargeOff.config.programs.scoot.desktop.power.chargeLimit.enable;
      true
    )
    # ...the M2's values as defaults (80% cap, auto-detect battery,
    # trip after 30 min on battery, back after a day on the
    # charger)...
    (
      assert hmPower.config.programs.scoot.desktop.power.chargeLimit.limit == 80;
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.chargeLimit.battery == null;
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.chargeLimit.fullAfter == 1800;
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.chargeLimit.tripEndsAfter == 86400;
      true
    )
    # ...the charge bounds as set (a lower cap on a named battery, an
    # hourly trip, a half-day trip end)...
    (
      assert hmPowerChargeBounds.config.programs.scoot.desktop.power.chargeLimit.limit == 70;
      true
    )
    (
      assert hmPowerChargeBounds.config.programs.scoot.desktop.power.chargeLimit.battery == "BAT0";
      true
    )
    (
      assert hmPowerChargeBounds.config.programs.scoot.desktop.power.chargeLimit.fullAfter == 3600;
      true
    )
    (
      assert hmPowerChargeBounds.config.programs.scoot.desktop.power.chargeLimit.tripEndsAfter == 43200;
      true
    )
    # ...no auto-switch by default (PPD holds whatever is set), the
    # lid and power-key actions as set, the PowerOff trip as set...
    (
      assert hmPower.config.programs.scoot.desktop.power.profileOnAC == null;
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.profileOnBattery == null;
      true
    )
    (
      assert hmPowerAuto.config.programs.scoot.desktop.power.profileOnAC == "performance";
      true
    )
    (
      assert hmPowerAuto.config.programs.scoot.desktop.power.profileOnBattery == "power-saver";
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.lidSwitch == "suspend";
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.lidSwitchDocked == "lock";
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.lidSwitchExternalPower == "suspend";
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.powerKey == "suspend";
      true
    )
    (
      assert hmPowerLid.config.programs.scoot.desktop.power.lidSwitch == "lock";
      true
    )
    (
      assert hmPowerLid.config.programs.scoot.desktop.power.powerKey == "ignore";
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.lowBattery.percentage == 2;
      true
    )
    (
      assert hmPower.config.programs.scoot.desktop.power.lowBattery.action == "Suspend";
      true
    )
    (
      assert hmPowerLowAction.config.programs.scoot.desktop.power.lowBattery.action == "PowerOff";
      true
    )
    # ...the daemon's package on PATH beside the switch script...
    (
      assert lib.any (p: (p.pname or "") == "power-profiles-daemon") hmPower.config.home.packages;
      true
    )
    (
      assert lib.any (p: (p.name or "") == "scoot-power-profile") hmPower.config.home.packages;
      true
    )
    # ...the fill unit beside the bar calls the system-wide
    # `scoot-charge` by absolute path (systemd resolves a bare
    # `ExecStart` through its own compile-time search path, never the
    # manager's PATH: a bare name fails every login with 203/EXEC)...
    (
      assert allAssertionsHold hmPowerBar.config;
      true
    )
    (
      assert
        hmPowerBar.config.systemd.user.services.scoot-charge-push.Service.ExecStart
        == "/run/current-system/sw/bin/scoot-charge push";
      true
    )
    # ...and without the bar module no fill unit renders...
    (
      assert !(hmPower.config.systemd.user.services ? scoot-charge-push);
      true
    )
    # ...and the profile bind beside the keymap's twenty-one (a
    # plain string: fire once, never while locked, like the other
    # slot binds)...
    (
      assert builtins.length (builtins.attrNames hmPower.config.programs.scoot.settings.binds) == 22;
      true
    )
    (
      assert lib.isString hmPower.config.programs.scoot.settings.binds."super+p";
      true
    )
    (
      assert lib.hasInfix "/bin/scoot-power-profile cycle"
        hmPower.config.programs.scoot.settings.binds."super+p";
      true
    )
    # ...while the profile without the policy stays at twenty-one
    # (the new bind is slot-gated, not keymap-owned).
    (
      assert builtins.length (builtins.attrNames hmKeys.config.programs.scoot.settings.binds) == 21;
      true
    )
    # Refusals: the policy with no daemon...
    (
      assert builtins.length (failing hmPowerNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "power.profiles.package is null" (builtins.head (failing hmPowerNoPkg.config));
      true
    )
    # ...a cap outside 1..100...
    (
      assert builtins.length (failing hmPowerBadLimit.config) == 1;
      true
    )
    (
      assert lib.hasInfix "chargeLimit.limit" (builtins.head (failing hmPowerBadLimit.config));
      true
    )
    # ...a blank battery name...
    (
      assert builtins.length (failing hmPowerBadBattery.config) == 1;
      true
    )
    (
      assert lib.hasInfix "chargeLimit.battery" (builtins.head (failing hmPowerBadBattery.config));
      true
    )
    # ...and a low-battery percent above UPower's critical.
    (
      assert builtins.length (failing hmPowerBadLowPct.config) == 1;
      true
    )
    (
      assert lib.hasInfix "lowBattery.percentage" (builtins.head (failing hmPowerBadLowPct.config));
      true
    )
    # ...and a bad charge bound with the policy off (the bounds sit
    # outside `power.enable`).
    (
      assert builtins.length (failing hmChargeBadLimitNoPower.config) == 1;
      true
    )
    (
      assert lib.hasInfix "chargeLimit.limit" (builtins.head (failing hmChargeBadLimitNoPower.config));
      true
    )
    # NixOS: every assertion holds, with the profile or standalone...
    (
      assert allAssertionsHold osPower.config;
      true
    )
    (
      assert allAssertionsHold osPowerStandalone.config;
      true
    )
    (
      assert allAssertionsHold osPowerChargeOff.config;
      true
    )
    (
      assert allAssertionsHold osPowerAuto.config;
      true
    )
    (
      assert allAssertionsHold osPowerLidOverride.config;
      true
    )
    (
      assert allAssertionsHold osPowerLowAction.config;
      true
    )
    (
      assert allAssertionsHold osPowerNoProfiles.config;
      true
    )
    (
      assert allAssertionsHold osPowerDockedSuspend.config;
      true
    )
    (
      assert allAssertionsHold osPowerServiceOverride.config;
      true
    )
    # ...the profiles daemon enabled, the flake's package behind it
    # (still additive: no default session, ever)...
    (
      assert osPower.config.services.power-profiles-daemon.enable;
      true
    )
    (
      assert
        osPower.config.services.power-profiles-daemon.package.drvPath == pkgs.power-profiles-daemon.drvPath;
      true
    )
    (
      assert osPower.config.services.displayManager.defaultSession == null;
      true
    )
    # ...the daemon wanted by default, droppable where no driver
    # honors it (the lid, low-battery and charge-limit policy still
    # applies)...
    (
      assert osPower.config.programs.scoot.desktop.power.profiles.enable;
      true
    )
    (
      assert !osPowerNoProfiles.config.services.power-profiles-daemon.enable;
      true
    )
    (
      assert
        !(lib.any (
          p: (p.pname or "") == "power-profiles-daemon"
        ) osPowerNoProfiles.config.environment.systemPackages);
      true
    )
    # ...while a user's own service values win over the policy's
    # defaults without an eval error...
    (
      assert !osPowerServiceOverride.config.services.power-profiles-daemon.enable;
      true
    )
    (
      assert !osPowerServiceOverride.config.services.upower.enable;
      true
    )
    # ...low battery through UPower (suspend at 2%, the risky-action
    # flag beside it -- s2idle, not the HybridSleep default that
    # would fail without persistent swap)...
    (
      assert osPower.config.services.upower.enable;
      true
    )
    (
      assert osPower.config.services.upower.percentageAction == 2;
      true
    )
    (
      assert osPower.config.services.upower.criticalPowerAction == "Suspend";
      true
    )
    (
      assert osPower.config.services.upower.allowRiskyCriticalPowerAction;
      true
    )
    # ...with a PowerOff trip the flag stays out...
    (
      assert osPowerLowAction.config.services.upower.criticalPowerAction == "PowerOff";
      true
    )
    (
      assert !(osPowerLowAction.config.services.upower ? allowRiskyCriticalPowerAction);
      true
    )
    # ...the lid and power-key actions on the canonical logind path
    # (docked locks, never suspends; logout leaves the user processes
    # a remote session shares this manager with to nixpkgs' own
    # default, which the real-NixOS pin below checks)...
    (
      assert osPower.config.services.logind.settings.Login.HandleLidSwitch == "suspend";
      true
    )
    (
      assert osPower.config.services.logind.settings.Login.HandleLidSwitchDocked == "lock";
      true
    )
    # ...while a user-set docked rule wins over the idle child's twin
    # (which defers while the policy runs)...
    (
      assert
        osPowerDockedSuspend.config.services.logind.settings.Login.HandleLidSwitchDocked == "suspend";
      true
    )
    (
      assert osPower.config.services.logind.settings.Login.HandleLidSwitchExternalPower == "suspend";
      true
    )
    (
      assert osPower.config.services.logind.settings.Login.HandlePowerKey == "suspend";
      true
    )
    (
      assert !(osPower.config.services.logind.settings.Login ? KillUserProcesses);
      true
    )
    # ...an explicit lid action winning over the policy default...
    (
      assert osPowerLidOverride.config.services.logind.settings.Login.HandleLidSwitch == "ignore";
      true
    )
    # ...the charge cap on with the policy (still individually
    # disable-able)...
    (
      assert osPower.config.programs.scoot.desktop.power.chargeLimit.enable;
      true
    )
    (
      assert !osPowerChargeOff.config.programs.scoot.desktop.power.chargeLimit.enable;
      true
    )
    # ...the daemon and the charge script installed beside the
    # profile's own system packages (exactly those two added)...
    (
      assert
        sorted osPower.config.environment.systemPackages == sorted (
          osDesk.config.environment.systemPackages
          ++ [
            pkgs.power-profiles-daemon
            (import ./modules/power-charge.nix {
              inherit pkgs lib;
              limit = 80;
              fullAfter = 1800;
              tripEndsAfter = 86400;
              battery = null;
            })
          ]
        );
      true
    )
    # ...standalone (no profile): the policy without the session
    # entry or its units...
    (
      assert osPowerStandalone.config.services.displayManager.sessionPackages == [ ];
      true
    )
    (
      assert osPowerStandalone.config.systemd.user.units == { };
      true
    )
    # ...the profile without the policy: no daemon, no UPower, no
    # charge units, no udev rules -- while the idle child's
    # docked-lid rule (which the policy twins when on) still lands.
    (
      assert osPowerOff.config.services.power-profiles-daemon == { };
      true
    )
    (
      assert osPowerOff.config.services.upower == { };
      true
    )
    (
      assert osPowerOff.config.services.logind.settings.Login == { HandleLidSwitchDocked = "lock"; };
      true
    )
    (
      assert !(osPowerOff.config.systemd.services ? scoot-charge-sync);
      true
    )
    (
      assert !(osPowerOff.config.systemd.timers ? scoot-charge-sync);
      true
    )
    (
      assert osPowerOff.config.services.udev.extraRules == "";
      true
    )
    (
      assert !osPowerOff.config.programs.scoot.desktop.power.chargeLimit.enable;
      true
    )
    # ...the charge cap off: the daemon and the lid policy without
    # the service, the timer or any udev rule...
    (
      assert !(osPowerChargeOff.config.systemd.services ? scoot-charge-sync);
      true
    )
    (
      assert !(osPowerChargeOff.config.systemd.timers ? scoot-charge-sync);
      true
    )
    (
      assert osPowerChargeOff.config.systemd.tmpfiles.rules == [ ];
      true
    )
    (
      assert osPowerChargeOff.config.services.udev.extraRules == "";
      true
    )
    # Refusals: the policy with no daemon, a cap outside 1..100, a
    # blank battery name, and a low-battery percent above UPower's
    # critical (each naming the option) -- plus a bad charge bound
    # with the policy off (the bounds sit outside `power.enable`).
    (
      assert builtins.length (failing osPowerNoPkg.config) == 1;
      true
    )
    (
      assert lib.hasInfix "power.profiles.package is null" (builtins.head (failing osPowerNoPkg.config));
      true
    )
    (
      assert builtins.length (failing osPowerBadLimit.config) == 1;
      true
    )
    (
      assert lib.hasInfix "chargeLimit.limit" (builtins.head (failing osPowerBadLimit.config));
      true
    )
    (
      assert builtins.length (failing osPowerBadBattery.config) == 1;
      true
    )
    (
      assert lib.hasInfix "chargeLimit.battery" (builtins.head (failing osPowerBadBattery.config));
      true
    )
    (
      assert builtins.length (failing osPowerBadLowPct.config) == 1;
      true
    )
    (
      assert lib.hasInfix "lowBattery.percentage" (builtins.head (failing osPowerBadLowPct.config));
      true
    )
    (
      assert builtins.length (failing osChargeBadLimitNoPower.config) == 1;
      true
    )
    (
      assert lib.hasInfix "chargeLimit.limit" (builtins.head (failing osChargeBadLimitNoPower.config));
      true
    )
  ];

  # --- power policy off Linux (fail `nix flake check` at eval) ---
  #
  # The daemon package is Linux-only like the idle policy's tools: off
  # Linux it defaults to null, which the assertion refuses loudly
  # instead of installing nothing silently. The logind, UPower and
  # charge values are plain data, so the docked-lid rule (and the
  # rest) still land. (The Linux check above is where the policy is
  # pinned.)
  _darwinPowerPins = lib.optionals (!isLinux) [
    # Home-manager: the daemon null...
    (
      assert hmPower.config.programs.scoot.desktop.power.profiles.package == null;
      true
    )
    # ...refused loudly beside the profile's other seventeen (the
    # idle policy's five, the daemon's one, the launcher's one, the
    # clipboard slot's three, the capture slot's four, the audio
    # slot's two, the night light's one).
    (
      assert builtins.length (failing hmPower.config) == 18;
      true
    )
    (
      assert lib.any (m: lib.hasInfix "power.profiles.package is null" m) (failing hmPower.config);
      true
    )
    # NixOS: the same null...
    (
      assert osPower.config.programs.scoot.desktop.power.profiles.package == null;
      true
    )
    # ...refused loudly there too (the idle policy's five plus the
    # daemon's one plus the launcher's one plus the clipboard slot's
    # three plus the capture slot's five on this side plus the audio
    # slot's two plus the night light's one), while the
    # docked-lid rule (plain values, no tools) still lands.
    (
      assert builtins.length (failing osPower.config) == 19;
      true
    )
    (
      assert osPower.config.services.logind.settings.Login.HandleLidSwitchDocked == "lock";
      true
    )
  ];
in
assert lib.all (x: x) _pins;
assert lib.all (x: x) _greeterPins;
assert lib.all (x: x) _desktopPins;
assert lib.all (x: x) _sessionPins;
assert lib.all (x: x) _startLimitPins;
assert lib.all (x: x) _idlePins;
assert lib.all (x: x) _notifPins;
assert lib.all (x: x) _clipPins;
assert lib.all (x: x) _launchPins;
assert lib.all (x: x) _capturePins;
assert lib.all (x: x) _audioPins;
assert lib.all (x: x) _nightlightPins;
assert lib.all (x: x) _keysPins;
assert lib.all (x: x) _powerPins;
assert lib.all (x: x) _darwinIdlePins;
assert lib.all (x: x) _darwinNotifPins;
assert lib.all (x: x) _darwinClipPins;
assert lib.all (x: x) _darwinLaunchPins;
assert lib.all (x: x) _darwinCapturePins;
assert lib.all (x: x) _darwinAudioPins;
assert lib.all (x: x) _darwinNightlightPins;
assert lib.all (x: x) _darwinKeysPins;
assert lib.all (x: x) _darwinPowerPins;
assert lib.all (x: x) _flakePins;
runCommand "scoot-modules-check" { nativeBuildInputs = [ checkPython ]; } ''
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
    #     quoted intact -- plus the sleep lock and the lock event, each
    #     wiping the clipboard history before `swaylock -f` with its
    #     config (the profile has the clipboard slot on). (Fixed-string
    #     matches throughout: the quoting is the assertion.)
    grep -F "timeout 120 '${pkgs.brightnessctl}/bin/brightnessctl -s set 10%' resume '${pkgs.brightnessctl}/bin/brightnessctl -r'" ${idleConf}
    grep -F "timeout 240 '${pkgs.systemd}/bin/loginctl lock-session'" ${idleConf}
    grep -F "timeout 300 '${pkgs.wlopm}/bin/wlopm --off \"*\"' resume '${pkgs.wlopm}/bin/wlopm --on \"*\"'" ${idleConf}
    grep -F "before-sleep '${leanClip}/bin/cliphist wipe; ${pkgs.swaylock}/bin/swaylock -f -C /nix/store/" ${idleConf}
    grep -F "lock '${leanClip}/bin/cliphist wipe; ${pkgs.swaylock}/bin/swaylock -f -C /nix/store/" ${idleConf}
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
    #      (and what the keymap's `super+escape` bind runs).
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

    # 13. Notification daemon, music-desk: the overlay layer (popups
    #     above fullscreen -- mako's own default is `top`, which the
    #     compositor hides), the DND section hiding, the look's roles
    #     as mako leaves (its `#rrggbb` colors kept verbatim), and the
    #     critical ring in urgent.
    grep -F -x "layer=overlay" ${notifConf}
    grep -F -x "[mode=do-not-disturb]" ${notifConf}
    grep -F -x "invisible=1" ${notifConf}
    grep -F -x "background-color=#FCFBFB" ${notifConf}
    grep -F -x "text-color=#1A2032" ${notifConf}
    grep -F -x "border-color=#3D579A" ${notifConf}
    grep -F -x "progress-color=over #3D579A" ${notifConf}
    grep -F -x "[urgency=critical]" ${notifConf}
    grep -F -x "border-color=#EE6F5E" ${notifConf}
    echo "ok: mako config carries the overlay layer, DND and the look"

    # 13b. A daemon setting wins per key (verbatim, leading `#` kept),
    #      a new key lands, and the generated sections stay.
    grep -F -x "layer=top" ${notifSettingsConf}
    if grep -q "^layer=overlay$" ${notifSettingsConf}; then echo "overridden default still present" >&2; exit 1; fi
    grep -F -x "anchor=bottom-right" ${notifSettingsConf}
    grep -F -x "background-color=#123456" ${notifSettingsConf}
    grep -F -x "[mode=do-not-disturb]" ${notifSettingsConf}
    grep -F -x "[urgency=critical]" ${notifSettingsConf}
    echo "ok: daemon settings win per key, generated sections stay"

    # 13c. Opted out (or lookless): no themed leaf at all -- with the
    #      opt-out the settings still apply, so the daemon is theirs.
    #      The DND section stays in both (it is behavior, not theme).
    if grep -q "^background-color=" ${notifTargetOffConf}; then echo "themed leaf present with theming off" >&2; exit 1; fi
    grep -F -x "border-color=#123456" ${notifTargetOffConf}
    grep -F -x "[mode=do-not-disturb]" ${notifTargetOffConf}
    if grep -q "background-color=" ${notifNoLookConf}; then echo "themed leaf present with no look" >&2; exit 1; fi
    if grep -q "urgency=critical" ${notifNoLookConf}; then echo "critical section present with no look" >&2; exit 1; fi
    grep -F -x "layer=overlay" ${notifNoLookConf}
    grep -F -x "[mode=do-not-disturb]" ${notifNoLookConf}
    echo "ok: opting out (or no look) leaves mako unthemed but hiding"

    # 13d. The bar feed: the push table carries the envelope icon and
    #      the DND toggle (the daemon's own command, absolute, so it
    #      works off PATH).
    python3 -c '
    import sys,tomllib
    got = tomllib.load(open(sys.argv[1],"rb"))
    assert got["push"]["notifications"]["icon"] == "○", got["push"]
    assert got["push"]["notifications"]["on-click"]["exec"] == [sys.argv[2], "mode", "-t", "do-not-disturb"], got["push"]
    ' ${notifBarToml} '${leanMako}/bin/makoctl'
    echo "ok: bar push module toggles do-not-disturb"

    # 13d2. Overridden state icons: the bar's static icon is the custom
      #       idle one (the envelope, back where it was), while the feed
      #       bridge carries the custom unread mark and an empty DND.
      python3 -c '
      import sys,tomllib
      got = tomllib.load(open(sys.argv[1],"rb"))
      assert got["push"]["notifications"]["icon"] == "✉", got["push"]
      ' ${notifIconsBarToml}
    grep -F -q -- "--arg icon_unread '!" ${feedIconsBridge}
    # An empty icon interpolates as two bare quotes: extract the DND
    # value and require it empty (spelling those quotes literally
    # would end this file's own string, so match around them).
    icon_dnd_val="$(grep -o -- "--arg icon_dnd '[^']*'" ${feedIconsBridge} | sed -e "s/^[^']*'//" -e "s/'$//")"
    [ -z "$icon_dnd_val" ] || { echo "want an empty DND icon (13d2), got: $icon_dnd_val"; exit 1; }
    echo "ok: overridden state icons reach the bar and the feed"

      # 13d3. By default the bridge carries the solid dot for unread and
      #       the crescent moon for DND.
      grep -F -q -- "--arg icon_unread '●'" ${feedBridge}
      grep -F -q -- "--arg icon_dnd '☾'" ${feedBridge}
      echo "ok: default state icons ride the feed"

    # 13e. The lean daemon: its runtime closure names no GTK stack
    #      (the `wrapGAppsHook3` weight the profile refuses to ship:
    #      gtk+3, tinysparql, cups, at-spi2-core, avahi). Any of those
    #      names reappearing -- a rebase silently restoring the hook,
    #      say -- fails loudly here.
    if grep -E "gtk\+3|tinysparql|cups|at-spi2|avahi" ${leanMakoClosure}/store-paths; then echo "GTK-stack path in lean mako closure" >&2; exit 1; fi
    echo "ok: lean mako closure carries no GTK stack"

    # 13f. ...while its wrapper still sets what icons need: the pixbuf
    #      loaders cache and an icon theme dir (both gtk-free: the
    #      cache names only gdk-pixbuf and librsvg).
    grep -q "GDK_PIXBUF_MODULE_FILE=" ${leanMako}/bin/mako
    grep -q "hicolor-icon-theme" ${leanMako}/bin/mako
    if grep -E "gtk\+3|tinysparql" ${leanMako}/bin/mako ${leanMako}/bin/makoctl; then echo "GTK-stack reference in lean mako wrapper" >&2; exit 1; fi
    echo "ok: lean mako wrapper sets the pixbuf loaders and the icon theme"

    # 13g. The default state icons are each at most one glyph, and all
    #      in DejaVu Sans (the bar's default font file), so they render
    #      with no symbol font. The icons come from the evaluated
    #      options, not pasted here, so a default no DejaVu ships (or a
    #      two-glyph one) fails loudly here instead of shipping a
    #      missing glyph.
    python3 -c '
    import sys
    from fontTools.ttLib import TTFont
    cmap = TTFont(sys.argv[1]).getBestCmap()
    icons = sys.argv[2:]
    long = [c for c in icons if len(c) > 1]
    assert not long, "state icons longer than one glyph: %s" % long
    missing = ["U+%04X" % ord(c) for c in "".join(icons) if ord(c) not in cmap]
    assert not missing, "state icons missing from DejaVu Sans: %s" % ", ".join(missing)
    ' ${pkgs.dejavu_fonts.minimal}/share/fonts/truetype/DejaVuSans.ttf '${hmNotif.config.programs.scoot.desktop.notifications.bar.icons.idle}' '${hmNotif.config.programs.scoot.desktop.notifications.bar.icons.unread}' '${hmNotif.config.programs.scoot.desktop.notifications.bar.icons.dnd}'
    echo "ok: default state icons are each one glyph in DejaVu Sans"

    # 14. The bar feed, against stub tools (the REAL bridge script from
    #     the module, scenario files below -- `mode`/`list` are what
    #     `makoctl` prints, `bus` what `busctl monitor` prints,
    #     `bar-err`/`bar-code` how `scootbar` answers, `calls` what it
    #     was asked). Every scenario asserts the exit status (bare:
    #     any other status fails the check) and the exact stderr lines.
    export SCOOT_FEED_TEST_DIR="$PWD/feed-test"
    mkdir -p "$SCOOT_FEED_TEST_DIR"
    feed_setup() {
      # $1 mode, $2 mode-code, $3 list, $4 list-code, $5 bar-err, $6 bar-code
      printf '%s' "$1" > "$SCOOT_FEED_TEST_DIR/mode"
      printf '%s' "$2" > "$SCOOT_FEED_TEST_DIR/mode-code"
      printf '%s' "$3" > "$SCOOT_FEED_TEST_DIR/list"
      printf '%s' "$4" > "$SCOOT_FEED_TEST_DIR/list-code"
      printf '%s' "$5" > "$SCOOT_FEED_TEST_DIR/bar-err"
      printf '%s' "$6" > "$SCOOT_FEED_TEST_DIR/bar-code"
      : > "$SCOOT_FEED_TEST_DIR/calls"
      : > "$SCOOT_FEED_TEST_DIR/stderr"
    }

    # 14a. The module not placed: the guidance names the missing
    #      module (not a dead bar), exactly one stderr line, exit 0.
    feed_setup 'default' 0 '[]' 0 'daemon: `notifications` is not placed in this bar (it shows: clock)' 1
    ${feedBridge} 2>"$SCOOT_FEED_TEST_DIR/stderr"
    [ "$(wc -l < "$SCOOT_FEED_TEST_DIR/stderr")" -eq 1 ] || { echo "want exactly one stderr line (14a)"; cat "$SCOOT_FEED_TEST_DIR/stderr"; exit 1; }
    grep -q "no notifications module" "$SCOOT_FEED_TEST_DIR/stderr"
    if grep -q "not running" "$SCOOT_FEED_TEST_DIR/stderr"; then echo "blamed the bar for an unplaced module (14a)" >&2; exit 1; fi
    grep -q '^msg set notifications' "$SCOOT_FEED_TEST_DIR/calls"
    echo "ok: feed names the unplaced module, once"

    # 14b. The bar not running: that is what the line says, once.
    feed_setup 'default' 0 '[]' 0 'scootbar: no scootbar daemon is running for eDP-1 (nothing listens on /run/user/1000/scootbar-eDP-1.sock); start one with `scootbar daemon`' 1
    ${feedBridge} 2>"$SCOOT_FEED_TEST_DIR/stderr"
    [ "$(wc -l < "$SCOOT_FEED_TEST_DIR/stderr")" -eq 1 ] || { echo "want exactly one stderr line (14b)"; cat "$SCOOT_FEED_TEST_DIR/stderr"; exit 1; }
    grep -q "cannot reach the bar" "$SCOOT_FEED_TEST_DIR/stderr"
    echo "ok: feed names the dead bar, once"

    # 14c. Malformed `makoctl list`: counts as empty, never as a
    #      crash -- exit 0, so `Restart=on-failure` does not respawn
    #      into a 2 s crash loop. One warning line; the push shows
    #      the empty bell.
    feed_setup 'default' 0 'this is not json' 0 "" 0
    ${feedBridge} 2>"$SCOOT_FEED_TEST_DIR/stderr"
    [ "$(wc -l < "$SCOOT_FEED_TEST_DIR/stderr")" -eq 1 ] || { echo "want exactly one stderr line (14c)"; cat "$SCOOT_FEED_TEST_DIR/stderr"; exit 1; }
    grep -q "malformed JSON" "$SCOOT_FEED_TEST_DIR/stderr"
    grep -qF '{"text":""}' "$SCOOT_FEED_TEST_DIR/calls"
    echo "ok: malformed list shows empty, no crash loop"

    # 14d. mako gone: the bar is cleared instead of left stale, one
    #      line saying so, exit 0.
    feed_setup "" 1 '[]' 0 "" 0
    ${feedBridge} 2>"$SCOOT_FEED_TEST_DIR/stderr"
    [ "$(wc -l < "$SCOOT_FEED_TEST_DIR/stderr")" -eq 1 ] || { echo "want exactly one stderr line (14d)"; cat "$SCOOT_FEED_TEST_DIR/stderr"; exit 1; }
    grep -q "cleared the bar" "$SCOOT_FEED_TEST_DIR/stderr"
    grep -qF '{"text":""}' "$SCOOT_FEED_TEST_DIR/calls"
    echo "ok: mako gone clears the bar"

    # 14e. `--watch`: losing the bus name clears the bar (the initial
    #      sync pushes the urgent count first). The monitor ending is
    #      exit 1, which is what restarts the feed.
    feed_setup 'default' 0 '[{"urgency": 2}]' 0 "" 0
    printf '%s\n' '{"type":"signal","sender":"org.freedesktop.DBus","path":"/org/freedesktop/DBus","interface":"org.freedesktop.DBus","member":"NameOwnerChanged","payload":{"type":"sss","data":["org.freedesktop.Notifications",":1.5",""]}}' > "$SCOOT_FEED_TEST_DIR/bus"
    if ${feedBridge} --watch 2>"$SCOOT_FEED_TEST_DIR/stderr"; then echo "monitor end should exit 1 (14e)" >&2; exit 1; fi
    grep -qF '"urgent"' "$SCOOT_FEED_TEST_DIR/calls"
    grep -qF '{"text":""}' "$SCOOT_FEED_TEST_DIR/calls"
    grep -q "cleared the bar" "$SCOOT_FEED_TEST_DIR/stderr"
    grep -q "bus monitor ended" "$SCOOT_FEED_TEST_DIR/stderr"
    echo "ok: bus-name loss clears the bar on watch"

    # 14f. `--watch`: a new owner re-syncs (no clear, the state is
    #      fresh from the restarted daemon).
    feed_setup 'default' 0 '[{"urgency": 1}]' 0 "" 0
    printf '%s\n' '{"type":"signal","sender":"org.freedesktop.DBus","path":"/org/freedesktop/DBus","interface":"org.freedesktop.DBus","member":"NameOwnerChanged","payload":{"type":"sss","data":["org.freedesktop.Notifications","",":1.9"]}}' > "$SCOOT_FEED_TEST_DIR/bus"
    if ${feedBridge} --watch 2>"$SCOOT_FEED_TEST_DIR/stderr"; then echo "monitor end should exit 1 (14f)" >&2; exit 1; fi
    [ "$(grep -c '^msg set notifications' "$SCOOT_FEED_TEST_DIR/calls")" = 2 ] || { echo "want initial sync plus re-sync (14f)"; cat "$SCOOT_FEED_TEST_DIR/calls"; exit 1; }
    if grep -q "cleared the bar" "$SCOOT_FEED_TEST_DIR/stderr"; then echo "re-sync cleared instead (14f)" >&2; exit 1; fi
    echo "ok: bus-name gain re-syncs on watch"

    # 14g. `--watch`: a monitor line that is not JSON is skipped,
    #      never fatal (the initial sync still lands).
    feed_setup 'default' 0 '[]' 0 "" 0
    printf '%s\n' 'this is not a bus message' > "$SCOOT_FEED_TEST_DIR/bus"
    if ${feedBridge} --watch 2>"$SCOOT_FEED_TEST_DIR/stderr"; then echo "monitor end should exit 1 (14g)" >&2; exit 1; fi
    [ "$(wc -l < "$SCOOT_FEED_TEST_DIR/stderr")" -eq 1 ] || { echo "want exactly the monitor-ended line (14g)"; cat "$SCOOT_FEED_TEST_DIR/stderr"; exit 1; }
    grep -q "bus monitor ended" "$SCOOT_FEED_TEST_DIR/stderr"
    grep -q '^msg set notifications' "$SCOOT_FEED_TEST_DIR/calls"
    echo "ok: garbage monitor line is skipped"

    # 14h. Unread: the count arrives with the state's icon (a
      #      per-update icon, so it overrides the static idle one while
      #      set) -- exact payload, one line.
      feed_setup 'default' 0 '[{"urgency": 1},{"urgency": 0}]' 0 "" 0
      ${feedBridge} 2>"$SCOOT_FEED_TEST_DIR/stderr"
      grep -qF '{"text":"2","class":"normal","tooltip":"2 notifications -- click to hold them with do-not-disturb","icon":"●"}' "$SCOOT_FEED_TEST_DIR/calls"
      echo "ok: unread count carries the unread icon"

      # 14i. DND: the state's icon rides along too (the crescent moon by
      #      default), beside the held count.
      feed_setup 'do-not-disturb' 0 '[{"urgency": 1}]' 0 "" 0
      ${feedBridge} 2>"$SCOOT_FEED_TEST_DIR/stderr"
      grep -qF '{"text":"DND 1","class":"muted","tooltip":"1 notifications held by do-not-disturb -- click to let them through","icon":"☾"}' "$SCOOT_FEED_TEST_DIR/calls"
      echo "ok: DND carries the DND icon"

      # 14j. Idle: no icon key at all (the static idle icon shows, set
      #      beside the module -- nothing to override it with).
      feed_setup 'default' 0 '[]' 0 "" 0
      ${feedBridge} 2>"$SCOOT_FEED_TEST_DIR/stderr"
      grep -qF '{"text":""}' "$SCOOT_FEED_TEST_DIR/calls"
      if grep -q '"icon"' "$SCOOT_FEED_TEST_DIR/calls"; then echo "idle payload names an icon (14j)"; cat "$SCOOT_FEED_TEST_DIR/calls"; exit 1; fi
      echo "ok: idle names no icon"

      # 14k. Overridden icons: the custom unread mark rides the count...
      feed_setup 'default' 0 '[{"urgency": 0}]' 0 "" 0
      ${feedIconsBridge} 2>"$SCOOT_FEED_TEST_DIR/stderr"
      grep -qF '{"text":"1","class":"normal","tooltip":"1 notifications -- click to hold them with do-not-disturb","icon":"!"}' "$SCOOT_FEED_TEST_DIR/calls"
      echo "ok: overridden unread icon rides the count"

      # 14l. ...and an empty DND icon stays out of the payload (the DND
      #      text alone).
      feed_setup 'do-not-disturb' 0 '[]' 0 "" 0
      ${feedIconsBridge} 2>"$SCOOT_FEED_TEST_DIR/stderr"
      grep -qF '{"text":"DND","class":"muted","tooltip":"do-not-disturb is on -- click to let notifications through"}' "$SCOOT_FEED_TEST_DIR/calls"
      if grep -q '"icon"' "$SCOOT_FEED_TEST_DIR/calls"; then echo "empty DND icon leaked into the payload (14l)"; cat "$SCOOT_FEED_TEST_DIR/calls"; exit 1; fi
      echo "ok: empty DND icon stays out of the payload"

    # 15. Keymap content: the rendered `[binds]` carries the
    #     twenty-one profile binds (the sigils -- `@...@`, `%`, `+` --
    #     intact through TOML; the notification binds through mako's
    #     absolute path, the daemon, the clipboard slot, the launcher
    #     slot and the capture slot being on with the profile -- the
    #     picker, both launcher binds and the three capture binds
    #     through their own scripts -- and the volume, brightness and
    #     mic-mute binds through the audio slot's scripts, on with the
    #     profile too),
    #     and with every slot on all twenty-one (slot scripts as store
    #     paths). No `wofi` anywhere: the launcher child reconciled the
    #     old default. The launcher binds stay plain strings (never
    #     repeat, never allowed while locked -- unlike the hardware
    #     tables above).
    python3 -c '
    import sys,tomllib
    got = tomllib.load(open(sys.argv[1],"rb"))["binds"]
    assert len(got) == 21, got.keys()
    assert "wofi" not in open(sys.argv[1]).read(), "wofi default left in [binds]"
    vol = got["XF86AudioRaiseVolume"]
    assert vol["action"].endswith("/bin/scoot-volume sink-up"), vol
    assert vol["repeat"] is True and vol["allow_when_locked"] is True, vol
    assert got["XF86AudioMute"]["action"].endswith("/bin/scoot-volume sink-mute"), got["XF86AudioMute"]
    assert got["XF86AudioMicMute"]["action"].endswith("/bin/scoot-volume mic-mute"), got["XF86AudioMicMute"]
    bri = got["XF86MonBrightnessUp"]
    assert bri["action"].endswith("/bin/scoot-brightness up"), bri
    assert bri["repeat"] is True and bri["allow_when_locked"] is True, bri
    assert got["super+escape"].endswith(" lock-session"), got["super+escape"]
    assert isinstance(got["super+v"], str), got["super+v"]
    assert "/bin/scoot-clipboard-pick" in got["super+v"], got["super+v"]
    assert isinstance(got["super+d"], str), got["super+d"]
    assert "/bin/scoot-launcher" in got["super+d"], got["super+d"]
    assert "--list-executables-in-path" not in got["super+d"], got["super+d"]
    assert isinstance(got["ctrl+alt+space"], str), got["ctrl+alt+space"]
    assert got["ctrl+alt+space"].endswith("/bin/scoot-launcher --list-executables-in-path"), got["ctrl+alt+space"]
    assert got["super+n"].startswith("spawn ") and got["super+n"].endswith("/bin/makoctl dismiss"), got["super+n"]
    assert got["super+shift+n"].endswith("/bin/makoctl mode -t do-not-disturb"), got["super+shift+n"]
    assert got["super+ctrl+n"].endswith("/bin/makoctl restore"), got["super+ctrl+n"]
    assert isinstance(got["print"], str), got["print"]
    assert "/bin/scoot-capture-output" in got["print"], got["print"]
    assert isinstance(got["shift+print"], str), got["shift+print"]
    assert "/bin/scoot-capture-region" in got["shift+print"], got["shift+print"]
    assert isinstance(got["ctrl+print"], str), got["ctrl+print"]
    assert "/bin/scoot-capture-clipboard" in got["ctrl+print"], got["ctrl+print"]
    ' ${keysToml}
    python3 -c '
    import sys,tomllib
    got = tomllib.load(open(sys.argv[1],"rb"))["binds"]
    assert len(got) == 21, got.keys()
    assert "wofi" not in open(sys.argv[1]).read(), "wofi default left in [binds]"
    assert "/bin/scoot-launcher" in got["super+d"] and "--list-executables-in-path" not in got["super+d"], got["super+d"]
    assert got["ctrl+alt+space"].endswith("/bin/scoot-launcher --list-executables-in-path"), got["ctrl+alt+space"]
    assert got["super+n"].endswith("/bin/makoctl dismiss"), got["super+n"]
    assert "/bin/scoot-clipboard-pick" in got["super+v"], got["super+v"]
    assert "/bin/scoot-volume sink-up" in got["XF86AudioRaiseVolume"]["action"], got["XF86AudioRaiseVolume"]
    assert "/bin/scoot-brightness up" in got["XF86MonBrightnessUp"]["action"], got["XF86MonBrightnessUp"]
    assert "/bin/scoot-capture-output" in got["print"], got["print"]
    assert "/bin/scoot-capture-region" in got["shift+print"], got["shift+print"]
    assert "/bin/scoot-capture-clipboard" in got["ctrl+print"], got["ctrl+print"]
    ' ${keysSlotsToml}
    # No `wofi` default left in the modules either (a dangling default
    # that spawns nothing): `clipboard-cliphist.nix` still names it
    # once, as the rejected picker fat the lean package drops -- that
    # comment is documentation, not a default, so it is excluded here.
    if grep -rn "wofi" ${./modules} | grep -v "clipboard-cliphist.nix"; then echo "wofi default left in nix/modules" >&2; exit 1; fi
    echo "ok: rendered [binds] carries the keymap (twenty-one with the daemon and the slots)"

    # 16. Clipboard slot content: the idle policy's lock lines carry
    #     the wipe (absolute cliphist path, before the locker, on both
    #     lock paths); with the slot off the lock lines stay wipe-free;
    #     the picker is themed by the look (music-desk roles as fuzzel
    #     CLI colors, opaque), unthemed without a look or opted out;
    #     the store entry bounds the history and guards the lock; the
    #     lean manager carries no picker fat.
    grep -F "lock '${leanClip}/bin/cliphist wipe; ${pkgs.swaylock}/bin/swaylock" ${idleConf}
    grep -F "before-sleep '${leanClip}/bin/cliphist wipe; ${pkgs.swaylock}/bin/swaylock" ${idleConf}
    echo "ok: the lock lines wipe the history before the locker"
    if grep -F "lock '" ${idleClipOffConf} | grep -q cliphist; then echo "wipe present with the slot off" >&2; exit 1; fi
    grep -F "lock '${pkgs.swaylock}/bin/swaylock" ${idleClipOffConf}
    echo "ok: with the slot off the lock lines stay wipe-free"

    # 16b. The picker, music-desk: the look's roles as fuzzel colors
    #      (opaque `rrggbbaa`), the dmenu contract flags, the lock
    #      probe, and the byte-exact restore (temp file, never command
    #      substitution -- which would strip trailing newlines).
    grep -F -q -- "--background-color=FCFBFBff" ${clipPickerThemed}
    grep -F -q -- "--text-color=1A2032ff" ${clipPickerThemed}
    grep -F -q -- "--border-color=3D579Aff" ${clipPickerThemed}
    grep -F -q -- "--selection-color=3D579Aff" ${clipPickerThemed}
    grep -F -q -- "--selection-text-color=FCFBFBff" ${clipPickerThemed}
    grep -F -q -- "--match-color=5D7AB0ff" ${clipPickerThemed}
    grep -F -q -- "--prompt-color=1A2032ff" ${clipPickerThemed}
    grep -F -q -- "--dmenu --prompt='clipboard: ' --no-run-if-empty --only-match" ${clipPickerThemed}
    grep -F -q "clipboard_unlocked" ${clipPickerThemed}
    grep -F -q "msg locked" ${clipPickerThemed}
    grep -F -q 'wl-copy <"$tmp"' ${clipPickerThemed}
    if grep -F -q 'wl-copy <<<' ${clipPickerThemed}; then echo "here-string restore (strips newlines)" >&2; exit 1; fi
    echo "ok: the picker carries the look, the dmenu contract and the lock probe"

    # 16c. Lookless (or opted out): no themed flag at all -- the probe
    #      and the dmenu contract stay (behavior, not theme).
    if grep -q -- "--background-color=" ${clipPickerNoLook}; then echo "themed flag present with no look" >&2; exit 1; fi
    grep -F -q -- "--dmenu --prompt='clipboard: '" ${clipPickerNoLook}
    grep -F -q "clipboard_unlocked" ${clipPickerNoLook}
    if grep -q -- "--background-color=" ${clipPickerTargetOff}; then echo "themed flag present with theming off" >&2; exit 1; fi
    grep -F -q "clipboard_unlocked" ${clipPickerTargetOff}
    echo "ok: opting out (or no look) leaves fuzzel unthemed but guarded"

    # 16c2. radial-burst: no `hover` in its bar palette, so the match
    #       highlight falls back to the accent (blue `31a9e5`).
    grep -F -q -- "--background-color=241721ff" ${clipPickerBurst}
    grep -F -q -- "--match-color=31a9e5ff" ${clipPickerBurst}
    echo "ok: a look without hover falls back to the accent"

    # 16d. The store entry: the lock probe first (fail-open without
    #      IPC, so a broken probe costs the lock guarantee rather than
    #      the history), then the bounded store (oldest dropped first
    #      past the cap).
    grep -F -q "clipboard_unlocked" ${clipEntry}
    grep -F -q "msg locked" ${clipEntry}
    grep -F -q -- "-max-items 100 store" ${clipEntry}
    if grep -q -- "-db-path" ${clipEntry}; then echo "db flag present with the default db" >&2; exit 1; fi
    grep -F -q -- "-max-items 250 store" ${clipBoundsEntry}
    grep -F -q -- "-db-path '/home/scoot-test/.cache/cliphist-test/db'" ${clipBoundsEntry}
    echo "ok: the store entry guards the lock and bounds the history"

    # 16d2. One moved db: the store entry, the picker (`list` and
    #       `decode`) and the lock wipe all name the same absolute path
    #       (the eval refusal above is what keeps a `~` or spaced path
    #       from reaching these lines naming different files).
    grep -F -q -- "-db-path '/home/scoot-test/.cache/cliphist-test/db' list" ${clipBoundsPicker}
    grep -F -q -- "-db-path '/home/scoot-test/.cache/cliphist-test/db' decode" ${clipBoundsPicker}
    grep -F "lock '${leanClip}/bin/cliphist -db-path '/home/scoot-test/.cache/cliphist-test/db' wipe; " ${idleClipBoundsConf}
    echo "ok: the store, the picker and the wipe name the same db"

    # 16e. The lean manager: its runtime closure names no picker fat
    #      (the contrib scripts' weight the profile refuses to ship:
    #      wofi and fuzzel, fzf and chafa, gtk+3 and its
    #      tinysparql/cups/at-spi2 train, perl, resvg). Any of those
    #      names reappearing -- a rebase silently restoring the
    #      scripts, say -- fails loudly here.
    if grep -E "wofi|fuzzel|fzf|chafa|gtk\+3|tinysparql|cups|at-spi2|avahi|resvg|perl-5" ${leanClipClosure}/store-paths; then echo "picker fat in lean cliphist closure" >&2; exit 1; fi
    echo "ok: lean cliphist closure carries no picker fat"

    # 17. The store entry and the picker, against stub tools (the REAL
    #     scripts from the modules, scenario files below -- `list` is
    #     what `cliphist list` prints, `pick`/`pick-code` how `fuzzel`
    #     answers, `decode-bytes`/`decode-code` how `cliphist decode`
    #     answers, `locked-code`/`locked-value` how the lock probe's
    #     `scoot` answers, `calls` what `cliphist` was asked,
    #     `menu-input` what the menu was offered, `pasted` what
    #     `wl-copy` received). Every scenario asserts the exit status
    #     and the exact resulting files.
    export SCOOT_CLIP_TEST_DIR="$PWD/clip-test"
    mkdir -p "$SCOOT_CLIP_TEST_DIR"
    clip_setup() {
      # $1 locked-code, $2 locked-value
      printf '%s' "$1" > "$SCOOT_CLIP_TEST_DIR/locked-code"
      printf '%s' "$2" > "$SCOOT_CLIP_TEST_DIR/locked-value"
      : > "$SCOOT_CLIP_TEST_DIR/calls"
      : > "$SCOOT_CLIP_TEST_DIR/menu-input"
      rm -f "$SCOOT_CLIP_TEST_DIR/pasted"
    }

    # 17a. Unlocked copy: the entry stores (argv, sensitivity and bytes
    #      all land in the call log), exit 0.
    clip_setup 0 false
    printf 'hello' > "$SCOOT_CLIP_TEST_DIR/stdin"
    printf 'data' > "$SCOOT_CLIP_TEST_DIR/state"
    CLIPBOARD_STATE="$(cat "$SCOOT_CLIP_TEST_DIR/state")" ${clipStoreEntry} < "$SCOOT_CLIP_TEST_DIR/stdin"
    grep -q "store-argv:.*store state:data" "$SCOOT_CLIP_TEST_DIR/calls"
    grep -q "^hello$" "$SCOOT_CLIP_TEST_DIR/calls"
    echo "ok: unlocked copy stores with its bytes and sensitivity"

    # 17b. Sensitive copy: the wrapper passes it through (cliphist
    #      itself skips the store -- pinned by its upstream suite and
    #      proved live with `wl-copy --sensitive`; the wrapper must not
    #      second-guess the state).
    clip_setup 0 false
    printf 's3cret' > "$SCOOT_CLIP_TEST_DIR/stdin"
    CLIPBOARD_STATE="sensitive" ${clipStoreEntry} < "$SCOOT_CLIP_TEST_DIR/stdin"
    grep -q "store-argv:.*store state:sensitive" "$SCOOT_CLIP_TEST_DIR/calls"
    echo "ok: sensitive copy reaches the store entry untouched"

    # 17c. Locked copy: nothing recorded, exit 0 (refuse-while-locked).
    clip_setup 0 true
    printf 'while-locked' > "$SCOOT_CLIP_TEST_DIR/stdin"
    CLIPBOARD_STATE="data" ${clipStoreEntry} < "$SCOOT_CLIP_TEST_DIR/stdin"
    [ ! -s "$SCOOT_CLIP_TEST_DIR/calls" ] || { echo "locked copy recorded"; cat "$SCOOT_CLIP_TEST_DIR/calls"; exit 1; }
    echo "ok: locked copy records nothing"

    # 17d. No IPC (no compositor yet): fail open and store, exit 0 -- a
    #      broken probe costs the lock guarantee, never the history.
    clip_setup 1 false
    printf 'no-ipc' > "$SCOOT_CLIP_TEST_DIR/stdin"
    CLIPBOARD_STATE="data" ${clipStoreEntry} < "$SCOOT_CLIP_TEST_DIR/stdin"
    grep -q "store-argv:.*store" "$SCOOT_CLIP_TEST_DIR/calls"
    echo "ok: without IPC the entry stores (fail-open)"

    # 17e. Both selections share the one entry (and so the one
    #      history): the primary unit runs the same script.
    [ "${clipPrimaryEntry}" = "${clipStoreEntry}" ]
    echo "ok: clipboard and primary share one history"

    # 17f. Pick the second of three: the menu is offered the whole
    #      list, `wl-copy` receives the decoded bytes byte-exact
    #      (trailing newlines intact -- the tempfile restore, not
    #      command substitution), exit 0.
    clip_setup 0 false
    printf '1\tfirst\n2\tsecond\n3\tthird\n' > "$SCOOT_CLIP_TEST_DIR/list"
    printf '2\tsecond' > "$SCOOT_CLIP_TEST_DIR/pick"
    printf '0' > "$SCOOT_CLIP_TEST_DIR/pick-code"
    printf 'SECOND-BYTES\nwith newline\n\n' > "$SCOOT_CLIP_TEST_DIR/decode-bytes"
    printf '0' > "$SCOOT_CLIP_TEST_DIR/decode-code"
    ${clipPicker}
    cmp "$SCOOT_CLIP_TEST_DIR/decode-bytes" "$SCOOT_CLIP_TEST_DIR/pasted"
    cmp "$SCOOT_CLIP_TEST_DIR/list" "$SCOOT_CLIP_TEST_DIR/menu-input"
    echo "ok: picking restores the decoded bytes exactly"

    # 17g. Cancelled pick (Escape): exit 0, the clipboard untouched (no
    #      `wl-copy` call at all -- with empty stdin it would *clear*
    #      the selection).
    clip_setup 0 false
    printf '1\tfirst\n' > "$SCOOT_CLIP_TEST_DIR/list"
    : > "$SCOOT_CLIP_TEST_DIR/pick"
    printf '1' > "$SCOOT_CLIP_TEST_DIR/pick-code"
    ${clipPicker}
    [ ! -e "$SCOOT_CLIP_TEST_DIR/pasted" ] || { echo "cancel cleared the clipboard"; exit 1; }
    if grep -q "decode-argv" "$SCOOT_CLIP_TEST_DIR/calls"; then echo "cancel decoded"; exit 1; fi
    echo "ok: a cancelled pick touches nothing"

    # 17h. Wiped mid-pick (the lock landed between list and decode):
    #      exit 0, the clipboard untouched.
    clip_setup 0 false
    printf '1\tfirst\n' > "$SCOOT_CLIP_TEST_DIR/list"
    printf '1\tfirst' > "$SCOOT_CLIP_TEST_DIR/pick"
    printf '0' > "$SCOOT_CLIP_TEST_DIR/pick-code"
    : > "$SCOOT_CLIP_TEST_DIR/decode-bytes"
    printf '1' > "$SCOOT_CLIP_TEST_DIR/decode-code"
    ${clipPicker}
    [ ! -e "$SCOOT_CLIP_TEST_DIR/pasted" ] || { echo "wiped pick pasted"; exit 1; }
    echo "ok: a pick wiped mid-flight pastes nothing"

    # 17i. Locked pick: refused naming the lock, exit 1, the clipboard
    #      untouched (the compositor suppresses the bind already; this
    #      covers a manual run).
    clip_setup 0 true
    printf '1\tfirst\n' > "$SCOOT_CLIP_TEST_DIR/list"
    printf '1\tfirst' > "$SCOOT_CLIP_TEST_DIR/pick"
    printf '0' > "$SCOOT_CLIP_TEST_DIR/pick-code"
    if ${clipPicker} 2>"$SCOOT_CLIP_TEST_DIR/stderr"; then echo "locked pick succeeded" >&2; exit 1; fi
    grep -q "locked" "$SCOOT_CLIP_TEST_DIR/stderr"
    [ ! -e "$SCOOT_CLIP_TEST_DIR/pasted" ] || { echo "locked pick pasted"; exit 1; }
    echo "ok: a locked pick is refused"

    # 18. The documented standalone desktop (site/desktop/index.md's
    #     home.nix snippet) renders the moonrise look end to end.
    python3 -c '
    import sys,tomllib
    got = tomllib.load(open(sys.argv[1],"rb"))
    assert got["appearance"] == {"background_color": "#2B3648", "focus_ring_active_color": "#FF9A49", "focus_ring_inactive_color": "#5E4B5B"}, got["appearance"]
    assert got["wallpaper"]["image"].endswith("moonrise.png"), got["wallpaper"]
    ' ${hmDocsDesktopToml}
    echo "ok: documented home-manager desktop renders moonrise"

    # 19. Launcher slot content: the wrapper script runs this
    #     profile's fuzzel by absolute store path (the one fuzzel the
    #     picker themes: same derivation, no second copy), on the
    #     `overlay` layer (above fullscreen -- fuzzel's own default is
    #     `top`, which the compositor hides under a fullscreen window),
    #     themed by the look (music-desk roles as fuzzel colors, opaque
    #     -- the same seven leaves the picker carries, one menu one
    #     palette), passing extra args through (which is what carries
    #     the run bind's `--list-executables-in-path`).
    grep -F -q -- "${pkgs.fuzzel}/bin/fuzzel --layer=overlay" ${launchThemed}
    grep -F -q -- "--background-color=FCFBFBff" ${launchThemed}
    grep -F -q -- "--text-color=1A2032ff" ${launchThemed}
    grep -F -q -- "--border-color=3D579Aff" ${launchThemed}
    grep -F -q -- "--selection-color=3D579Aff" ${launchThemed}
    grep -F -q -- "--selection-text-color=FCFBFBff" ${launchThemed}
    grep -F -q -- "--match-color=5D7AB0ff" ${launchThemed}
    grep -F -q -- "--prompt-color=1A2032ff" ${launchThemed}
    grep -F -q -- '"$@"' ${launchThemed}
    if grep -q -- "--dmenu" ${launchThemed}; then echo "dmenu flag in the launcher script" >&2; exit 1; fi
    if grep -q "clipboard_unlocked" ${launchThemed}; then echo "lock probe in the launcher script" >&2; exit 1; fi
    echo "ok: the launcher runs fuzzel on overlay, themed, passing args through"

    # 19b. Every look themes it (music-desk above; the other three
    #      here: each background role as its opaque flag).
    grep -F -q -- "--background-color=271A1Fff" ${launchVinyl}
    grep -F -q -- "--text-color=F1E3C6ff" ${launchVinyl}
    grep -F -q -- "--background-color=241721ff" ${launchBurst}
    grep -F -q -- "--match-color=31a9e5ff" ${launchBurst}
    grep -F -q -- "--background-color=2B3648ff" ${launchMoon}
    grep -F -q -- "--text-color=F6EEDCff" ${launchMoon}
    echo "ok: all four looks theme the launcher"

    # 19c. Lookless (or opted out): no themed flag at all -- the
    #      layer and the passthrough stay (behavior, not theme), and
    #      fuzzel's own colors stand.
    if grep -q -- "--background-color=" ${launchNoLook}; then echo "themed flag present with no look" >&2; exit 1; fi
    grep -F -q -- "${pkgs.fuzzel}/bin/fuzzel --layer=overlay" ${launchNoLook}
    grep -F -q -- '"$@"' ${launchNoLook}
    if grep -q -- "--background-color=" ${launchTargetOff}; then echo "themed flag present with theming off" >&2; exit 1; fi
    grep -F -q -- "${pkgs.fuzzel}/bin/fuzzel --layer=overlay" ${launchTargetOff}
    echo "ok: opting out (or no look) leaves fuzzel unthemed but layered"

    # 20. Capture slot content: the per-desktop chooser file xdpw
    #     reads first -- a dmenu ask through a wrapper script (a bare
    #     store path: xdpw's inih config reader cuts 200+ character
    #     lines mid-flag, so the theme travels inside the script,
    #     where no line limit applies), capped at 30 fps, with no
    #     fixed output beside a picker. The script runs this
    #     profile's fuzzel by absolute store path, themed by the look
    #     (music-desk roles as fuzzel colors, opaque -- the same
    #     seven leaves the launcher carries, one menu one palette).
    grep -F -q -- "[screencast]" ${captureThemed}
    grep -F -q -- "chooser_type=dmenu" ${captureThemed}
    grep -F -q -- "chooser_cmd=" ${captureThemed}
    grep -F -q -- "max_fps=30" ${captureThemed}
    if grep -q -- "output_name" ${captureThemed}; then echo "output_name present beside a chooser" >&2; exit 1; fi
    chooserScript=$(grep '^chooser_cmd=' ${captureThemed} | cut -d= -f2-)
    grep -F -q -- "${pkgs.fuzzel}/bin/fuzzel --dmenu --prompt='Share: '" "''${chooserScript}"
    grep -F -q -- "--background-color=FCFBFBff" "''${chooserScript}"
    grep -F -q -- "--text-color=1A2032ff" "''${chooserScript}"
    grep -F -q -- "--selection-color=3D579Aff" "''${chooserScript}"
    echo "ok: the chooser is a themed dmenu list at 30 fps"

    # 20b. Lookless (or opted out): the menu and the behavior stay,
    #      the theme leaves -- and the slurp pickers stay unthemed
    #      there too (no look to theme from: the region script
    #      carries no color flags).
    if grep -q -- "--background-color=" ${captureNoLook}; then echo "themed flag present with no look" >&2; exit 1; fi
    grep -F -q -- "chooser_type=dmenu" ${captureNoLook}
    chooserNoLook=$(grep '^chooser_cmd=' ${captureNoLook} | cut -d= -f2-)
    grep -F -q -- "${pkgs.fuzzel}/bin/fuzzel --dmenu" "''${chooserNoLook}"
    if grep -q -- "--background-color=" "''${chooserNoLook}"; then echo "themed flag present with no look" >&2; exit 1; fi
    if grep -q -- "--background-color=" ${captureTargetOff}; then echo "themed flag present with theming off" >&2; exit 1; fi
    grep -F -q -- "chooser_type=dmenu" ${captureTargetOff}
    if grep -q -- " -c '" ${captureRegionNoLook}; then echo "slurp color flag present with no look" >&2; exit 1; fi
    grep -F -q -- "${pkgs.slurp}/bin/slurp" ${captureRegionNoLook}
    echo "ok: opting out (or no look) leaves the chooser and the region picker unthemed"

    # 20c. The slurp picker instead: xdpw's own click-to-pick shape
    #      (its man page's example line), the look on its border and
    #      selection.
    grep -F -q -- "chooser_type=simple" ${captureSlurp}
    grep -F -q -- "${pkgs.slurp}/bin/slurp -f 'Monitor: %o' -or" ${captureSlurp}
    grep -F -q -- " -c '#3D579Aff'" ${captureSlurp}
    grep -F -q -- " -s '#3D579Aff'" ${captureSlurp}
    echo "ok: the slurp chooser clicks a screen, themed"

    # 20d. No picker: the fixed output, or any output -- and no
    #      chooser command beside `none`.
    grep -F -q -- "chooser_type=none" ${captureNoneNamed}
    grep -F -q -- "output_name=DP-1" ${captureNoneNamed}
    if grep -q -- "chooser_cmd" ${captureNoneNamed}; then echo "chooser_cmd present with chooser none" >&2; exit 1; fi
    grep -F -q -- "chooser_type=none" ${captureNoneAny}
    if grep -q -- "output_name" ${captureNoneAny}; then echo "output_name present for any output" >&2; exit 1; fi
    if grep -q -- "chooser_cmd" ${captureNoneAny}; then echo "chooser_cmd present with chooser none" >&2; exit 1; fi
    echo "ok: chooser none casts the fixed output, or any"

    # 20e. The screenshot scripts run the slot's tools by absolute
    #      store path: every output to a dated file under
    #      `~/Pictures`, a slurp-picked region beside it (the look on
    #      its border and selection), a picked region into the
    #      clipboard.
    grep -F -q -- "${pkgs.grim}/bin/grim" ${captureOutputScript}
    grep -F -q -- '"$HOME/Pictures"' ${captureOutputScript}
    grep -F -q -- "${pkgs.grim}/bin/grim -g" ${captureRegionScript}
    grep -F -q -- '$(${pkgs.slurp}/bin/slurp' ${captureRegionScript}
    grep -F -q -- " -c '#3D579Aff'" ${captureRegionScript}
    grep -F -q -- " -s '#3D579Aff'" ${captureRegionScript}
    grep -F -q -- "| ${pkgs.wl-clipboard}/bin/wl-copy" ${captureClipboardScript}
    echo "ok: the screenshot scripts name the slot's tools absolutely"

    # 20f. Session identity for screen sharing: every login -- greeter
    #      or console -- exports the Wayland session type (the
    #      launcher's harness T15 proves it past an inherited `tty`),
    #      so Chrome picks its portal capturer instead of X11 and Meet
    #      shares out of the box. The export (set, not defaulted: the
    #      session being started is always Wayland), the scoped
    #      manager+bus import, and the exit-time restore.
    grep -F -q -- "export XDG_SESSION_TYPE=wayland" ${../resources/scoot-session}
    grep -F -q -- "dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE" ${../resources/scoot-session}
    grep -F -q -- "import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE" ${../resources/scoot-session}
    grep -F -q -- "unset-environment XDG_SESSION_TYPE" ${../resources/scoot-session}
    echo "ok: the launcher exports the Wayland session type to the manager and the bus"

    # 20g. The login-screen entry is a Wayland session naming scoot:
    #      the directory is what tells the greeter the session type
    #      (greetd derives `XDG_SESSION_TYPE=wayland` from
    #      `wayland-sessions`), and `DesktopNames` is what names the
    #      desktop for the bus and the portal lookup.
    grep -F -q -- "share/wayland-sessions/scoot.desktop" ${../nix/modules/nixos.nix}
    grep -F -q -- "DesktopNames=scoot" ${../nix/modules/nixos.nix}
    echo "ok: the session entry is a wayland-sessions entry for scoot"
    # 21. Power policy content: the switch script wraps this
    #     profile's powerprofilesctl by absolute store path and
    #     rotates in PPD's canonical order (power-saver, balanced,
    #     performance), skipping whatever the daemon does not list;
    #     the keymap's profile bind is the twenty-second
    #     bind with the policy on (a plain string: fire once, never
    #     while locked, like the other slot binds).
    grep -F -q -- "${pkgs.power-profiles-daemon}/bin/powerprofilesctl" ${powerSwitchScript}
    grep -F -q -- 'power-saver) order="balanced performance power-saver"' ${powerSwitchScript}
    grep -F -q -- 'balanced) order="performance power-saver balanced"' ${powerSwitchScript}
    grep -F -q -- 'performance) order="power-saver balanced performance"' ${powerSwitchScript}
    grep -F -q -- '"$ctl" list' ${powerSwitchScript}
    grep -F -q -- 'power-saver\|balanced\|performance' ${powerSwitchScript}
    echo "ok: the switch wraps powerprofilesctl and rotates"
    python3 -c '
    import sys,tomllib
    got = tomllib.load(open(sys.argv[1],"rb"))["binds"]
    assert len(got) == 22, got.keys()
    assert isinstance(got["super+p"], str), got["super+p"]
    assert got["super+p"].endswith("/bin/scoot-power-profile cycle"), got["super+p"]
    ' ${keysPowerToml}
    echo "ok: the profile bind cycles the daemon on super+p"

    # 21b. The charge service, timer and udev rules: the unit runs
    #      `scoot-charge sync` as a oneshot; the timer re-syncs five
    #      minutes after boot and activity; tmpfiles owns the state
    #      dir for the users group; udev re-applies the group write
    #      on whatever supply owns a threshold node and re-runs the
    #      policy on every AC change (no per-machine kernel names in
    #      the rules -- the timer catches a missed event either way).
    printf '%s' '${chargeService.serviceConfig.ExecStart}' | grep -F -q -- "/bin/scoot-charge sync"
    test "${chargeService.serviceConfig.Type}" = oneshot
    test "${chargeTimer.timerConfig.OnBootSec}" = 1min
    test "${chargeTimer.timerConfig.OnUnitActiveSec}" = 5min
    printf '%s' '${lib.concatStringsSep "\n" chargeTmpfiles}' | grep -F -q -- "d /var/lib/scoot-charge 2775 root users -"
    printf '%s' '${chargeUdev}' | grep -F -q -- 'TEST=="charge_control_end_threshold"'
    printf '%s' '${chargeUdev}' | grep -F -q -- 'ENV{POWER_SUPPLY_TYPE}=="Mains"'
    echo "ok: the charge service, timer and udev rules"

    # 21c. The opt-in auto-switch: each set half adds its udev rule
    #      (AC online selects the AC profile, offline the battery
    #      one); without either half no rule selects a profile.
    printf '%s' '${chargeAutoUdev}' | grep -F -q -- 'POWER_SUPPLY_ONLINE}=="1"'
    printf '%s' '${chargeAutoUdev}' | grep -F -q -- "set performance"
    printf '%s' '${chargeAutoUdev}' | grep -F -q -- 'POWER_SUPPLY_ONLINE}=="0"'
    printf '%s' '${chargeAutoUdev}' | grep -F -q -- "set power-saver"
    if printf '%s' '${chargeUdev}' | grep -F -q -- "powerprofilesctl set"; then echo "auto-switch rule without opt-in (21c)" >&2; exit 1; fi
    echo "ok: auto-switch is opt-in per power state"

    # 21d. The switch, against stub powerprofilesctl (the REAL script
    #      from the keymap; scenario files below -- `current` is what
    #      `get` prints until a `set` lands, `sets` what `set` was
    #      asked, both codes how it answers, `list` the profiles the
    #      daemon offers in its own format, which `cycle` steps
    #      between).
    export SCOOT_POWER_TEST_DIR="$PWD/power-test"
    mkdir -p "$SCOOT_POWER_TEST_DIR"
    # All three (driver hardware), or the placeholder's two
    # (power-saver + balanced -- Apple silicon, no driver).
    printf '%s\n' '* balanced:' '    Driver: placeholder' '  power-saver:' '    Driver: placeholder' '  performance:' '    Driver: placeholder' > "$SCOOT_POWER_TEST_DIR/list-all"
    printf '%s\n' '* balanced:' '    Driver: placeholder' '  power-saver:' '    Driver: placeholder' > "$SCOOT_POWER_TEST_DIR/list-placeholder"
    power_setup() {
      # $1 current, $2 get-code, $3 set-code, $4 list file (all three
      # by default)
      printf '%s' "$1" > "$SCOOT_POWER_TEST_DIR/current"
      printf '%s' "$2" > "$SCOOT_POWER_TEST_DIR/get-code"
      printf '%s' "$3" > "$SCOOT_POWER_TEST_DIR/set-code"
      cp "''${4:-$SCOOT_POWER_TEST_DIR/list-all}" "$SCOOT_POWER_TEST_DIR/list"
      : > "$SCOOT_POWER_TEST_DIR/sets"
    }
    # balanced cycles to performance, printing it...
    power_setup 'balanced' 0 0
    switched="$(${powerSwitch} cycle)"; test "$switched" = performance
    grep -q -F -x performance "$SCOOT_POWER_TEST_DIR/sets"
    echo "ok: balanced cycles to performance"
    # ...performance to power-saver, power-saver to balanced, an
    # unknown answer back to balanced...
    power_setup 'performance' 0 0
    switched="$(${powerSwitch} cycle)"; test "$switched" = power-saver
    power_setup 'power-saver' 0 0
    switched="$(${powerSwitch} cycle)"; test "$switched" = balanced
    power_setup 'quiet' 0 0
    switched="$(${powerSwitch} cycle)"; test "$switched" = balanced
    grep -q -F -x balanced "$SCOOT_POWER_TEST_DIR/sets"
    echo "ok: the rotation covers every state"
    # ...a daemon listing two profiles cycles between those two
    # (the placeholder: stepping to performance would fail, and
    # from the keybind that failure would be silent)...
    power_setup 'balanced' 0 0 "$SCOOT_POWER_TEST_DIR/list-placeholder"
    switched="$(${powerSwitch} cycle)"; test "$switched" = power-saver
    test "$(cat "$SCOOT_POWER_TEST_DIR/sets")" = power-saver
    echo "ok: the cycle skips profiles the daemon does not list"
    power_setup 'power-saver' 0 0 "$SCOOT_POWER_TEST_DIR/list-placeholder"
    switched="$(${powerSwitch} cycle)"; test "$switched" = balanced
    echo "ok: the placeholder cycles power-saver to balanced"
    # ...an unreadable list falls back to the full rotation (loud,
    # like before)...
    power_setup 'balanced' 0 0 /dev/null
    switched="$(${powerSwitch} cycle)"; test "$switched" = performance
    grep -q -F -x performance "$SCOOT_POWER_TEST_DIR/sets"
    echo "ok: no list falls back to the full rotation"
    # ...a failed set fails the cycle (no silent landing)...
    power_setup 'balanced' 0 1
    if switched="$(${powerSwitch} cycle)"; then echo "failed set cycled (21d)" >&2; exit 1; fi
    echo "ok: a failed set fails the cycle"
    # ...status prints the daemon's answer, set takes exactly the
    # three, anything else is usage (exit 2).
    power_setup 'power-saver' 0 0
    switched="$(${powerSwitch})"; test "$switched" = power-saver
    switched="$(${powerSwitch} status)"; test "$switched" = power-saver
    ${powerSwitch} set balanced
    grep -q -F -x balanced "$SCOOT_POWER_TEST_DIR/sets"
    if ${powerSwitch} set turbo; then echo "bogus profile accepted (21d)" >&2; exit 1; fi
    if ${powerSwitch} frobnicate; then echo "bogus verb accepted (21d)" >&2; exit 1; fi
    echo "ok: status prints, set refuses the rest"

    # 21e. The charge policy, against a fake sysfs (the REAL script
    #      from the service -- `SCOOT_CHARGE_*` override the battery,
    #      the AC file and the state dir, so the check needs no
    #      hardware).
    export SCOOT_CHARGE_STATE="$PWD/charge-test/state"
    BAT="$PWD/charge-test/sys/class/power_supply/macsmc-battery"
    mkdir -p "$BAT" "$SCOOT_CHARGE_STATE"
    export SCOOT_CHARGE_BATTERY="$BAT"
    export SCOOT_CHARGE_AC="$PWD/charge-test/sys/class/power_supply/macsmc-ac/online"
    mkdir -p "$(dirname "$SCOOT_CHARGE_AC")"
    echo 80 > "$BAT/charge_control_end_threshold"
    echo 75 > "$BAT/capacity"
    echo 1 > "$SCOOT_CHARGE_AC"
    # the limit holds on AC (the first sync baselines, changing
    # nothing)...
    ${chargeScript} sync
    test "$(cat "$BAT/charge_control_end_threshold")" = 80
    test "$(cat "$SCOOT_CHARGE_STATE/last_ac")" = 1
    echo "ok: the limit holds on AC"
    # ...toggle charges full once...
    ${chargeScript} toggle
    test "$(cat "$SCOOT_CHARGE_STATE/mode")" = once
    test "$(cat "$BAT/charge_control_end_threshold")" = 100
    echo "ok: toggle charges full once"
    # ...unplug ends the once (back to the limit in the same sync)...
    echo 0 > "$SCOOT_CHARGE_AC"
    ${chargeScript} sync
    test "$(cat "$SCOOT_CHARGE_STATE/mode")" = limit
    test "$(cat "$BAT/charge_control_end_threshold")" = 80
    echo "ok: unplug ends full-once"
    # ...thirty minutes on battery trips the refill on replug...
    echo $(( $(date +%s) - 2000 )) > "$SCOOT_CHARGE_STATE/unplugged_at"
    echo 1 > "$SCOOT_CHARGE_AC"
    ${chargeScript} sync
    test "$(cat "$SCOOT_CHARGE_STATE/mode")" = trip
    test "$(cat "$BAT/charge_control_end_threshold")" = 100
    echo "ok: time on battery trips the refill"
    # ...a day on the charger ends the trip...
    echo $(( $(date +%s) - 90000 )) > "$SCOOT_CHARGE_STATE/plugged_at"
    ${chargeScript} sync
    test "$(cat "$SCOOT_CHARGE_STATE/mode")" = limit
    test "$(cat "$BAT/charge_control_end_threshold")" = 80
    echo "ok: a day on the charger ends the trip"
    # ...without the trip (fullAfter = 0) the refill never comes...
    export SCOOT_CHARGE_STATE="$PWD/charge-test/state-no-trip"
    mkdir -p "$SCOOT_CHARGE_STATE"
    echo 0 > "$SCOOT_CHARGE_AC"
    ${chargeScriptNoTrip}/bin/scoot-charge sync
    echo $(( $(date +%s) - 2000 )) > "$SCOOT_CHARGE_STATE/unplugged_at"
    echo 1 > "$SCOOT_CHARGE_AC"
    ${chargeScriptNoTrip}/bin/scoot-charge sync
    # never written: the implicit default is the limit (no trip was
    # armed, so there is no mode to persist)
    test "$(cat "$SCOOT_CHARGE_STATE/mode" 2>/dev/null || echo limit)" = limit
    test "$(cat "$BAT/charge_control_end_threshold")" = 80
    echo "ok: fullAfter = 0 disables the trip"
    # ...status reports the whole state on one line...
    export SCOOT_CHARGE_STATE="$PWD/charge-test/state"
    ${chargeScript} status > "$PWD/charge-test/status"
    grep -F -x -q "mode=limit threshold=80 ac=1 capacity=75%" "$PWD/charge-test/status"
    echo "ok: status reports the state"
    # ...without the sysfs node the service is inert, never
    # refused...
    export SCOOT_CHARGE_BATTERY="$PWD/charge-test/empty-battery"
    mkdir -p "$SCOOT_CHARGE_BATTERY"
    ${chargeScript} sync
    ${chargeScript} status > "$PWD/charge-test/status-empty"
    grep -F -q "threshold=unsupported" "$PWD/charge-test/status-empty"
    echo "ok: no node is inert, not refused"
    # ...and anything else is usage (exit 2).
    if ${chargeScript} frobnicate; then echo "bogus charge verb accepted (21e)" >&2; exit 1; fi
    echo "ok: usage refuses unknown verbs"

    # 22. Audio slot content: the wob config -- geometry, the hide
    #     timeout and the overflow clamp, the look's roles as wob
    #     colors (opaque `RRGGBBAA`), a washed muted style and an
    #     urgent overflow style.
    grep -F -x "timeout = 1500" ${audioThemed}
    grep -F -x "max = 100" ${audioThemed}
    grep -F -x "width = 400" ${audioThemed}
    grep -F -x "height = 40" ${audioThemed}
    grep -F -x "anchor = bottom" ${audioThemed}
    grep -F -x "margin = 48" ${audioThemed}
    grep -F -x "overflow_mode = nowrap" ${audioThemed}
    grep -F -x "background_color = FCFBFBff" ${audioThemed}
    grep -F -x "bar_color = 3D579Aff" ${audioThemed}
    grep -F -x "border_color = 3D579Aff" ${audioThemed}
    grep -F -x "overflow_bar_color = EE6F5Eff" ${audioThemed}
    grep -F -q -- "[style.muted]" ${audioThemed}
    grep -F -x "bar_color = C9CBD0ff" ${audioThemed}
    echo "ok: the OSD config carries the geometry, the timeout and the look"

    # 22b. Lookless (or opted out): geometry and the timeout stay, the
    #      look's colors leave (wob's own stand), the muted style still
    #      defined so mute never names a missing style.
    grep -F -x "timeout = 1500" ${audioNoLook}
    if grep -q "FCFBFBff" ${audioNoLook}; then echo "look color present with no look (22b)" >&2; exit 1; fi
    grep -F -q -- "[style.muted]" ${audioNoLook}
    grep -F -x "bar_color = 888888ff" ${audioNoLook}
    if grep -q "FCFBFBff" ${audioTargetOff}; then echo "look color present with theming off (22b)" >&2; exit 1; fi
    grep -F -q -- "[style.muted]" ${audioTargetOff}
    echo "ok: opting out (or no look) leaves wob unthemed but mute-safe"

    # 22c. Retimed: the override lands.
    grep -F -x "timeout = 2500" ${audioTimeoutIni}
    echo "ok: the retimed OSD renders its override"

    # 22d. The control scripts name the slot's tools by absolute store
    #      path and show through the one OSD script.
    grep -F -q -- "bin/wpctl set-volume" ${audioBehaviorVolume}
    grep -F -q -- "bin/wpctl set-mute" ${audioBehaviorVolume}
    grep -F -q -- "bin/wpctl get-volume" ${audioBehaviorVolume}
    grep -F -q -- "bin/scoot-osd show" ${audioBehaviorVolume}
    grep -F -q -- "bin/brightnessctl" ${audioBehaviorBrightness}
    grep -F -q -- "bin/scoot-osd show" ${audioBehaviorBrightness}
    grep -F -q -- "bin/pw-dump" ${audioBehaviorSink}
    grep -F -q -- "bin/wpctl set-default" ${audioBehaviorSink}
    grep -F -q -- "bin/scoot-volume sink-show" ${audioBehaviorSink}
    echo "ok: the control scripts name the slot's tools absolutely"

    # 22e. Volume, brightness and sink behavior, against stub tools
    #      (scripted through files under `$SCOOT_AUDIO_TEST_DIR`, one
    #      evaluation covering every scenario: the stub `wpctl`
    #      answers `get-volume` from `vol` (exit `vol-code`),
    #      records `set-volume`/`set-mute` in `calls`
    #      (exit `set-code`), answers `inspect` from `inspect` and
    #      records `set-default` in `calls` plus `default-id`; the
    #      stub `brightnessctl` lists `devs` and records sets; the
    #      stub `pw-dump` prints `dump`).
    export SCOOT_AUDIO_TEST_DIR="$PWD/audio-test"
    mkdir -p "$SCOOT_AUDIO_TEST_DIR"
    audio_setup() {
      printf '%s' "$1" > "$SCOOT_AUDIO_TEST_DIR/vol"
      printf '%s' "$2" > "$SCOOT_AUDIO_TEST_DIR/vol-code"
      printf '%s' "$3" > "$SCOOT_AUDIO_TEST_DIR/set-code"
      printf '%s' "$4" > "$SCOOT_AUDIO_TEST_DIR/devs"
      printf '%s' "$5" > "$SCOOT_AUDIO_TEST_DIR/dump"
      printf '%s' "$6" > "$SCOOT_AUDIO_TEST_DIR/dump-code"
      printf '%s' "$7" > "$SCOOT_AUDIO_TEST_DIR/inspect"
      : > "$SCOOT_AUDIO_TEST_DIR/calls"
    }
    audio_fifo() {
      export XDG_RUNTIME_DIR="$SCOOT_AUDIO_TEST_DIR/rt"
      mkdir -p "$XDG_RUNTIME_DIR"
      rm -f "$XDG_RUNTIME_DIR/scoot-osd.fifo"
      mkfifo "$XDG_RUNTIME_DIR/scoot-osd.fifo"
    }
    audio_shown() {
      # Drain one OSD line. The fifo stays held open read-write for
      # the whole exchange (opening either end never blocks while the
      # other end is held): without the hold, a writer that finishes
      # before the reader's read-only open leaves that open blocking
      # forever, hanging the check under load. The reader's own
      # status is meaningless (only the writer's is reported), and it
      # runs under a timeout so even a stuck reader fails the
      # assertion loudly instead of hanging the suite.
      exec 3<>"$XDG_RUNTIME_DIR/scoot-osd.fifo"
      timeout 10 cat <&3 > "$SCOOT_AUDIO_TEST_DIR/shown" &
      reader=$!
      "$@"
      status=$?
      exec 3>&-
      wait "$reader" || true
      printf '%s:' "$status"
      cat "$SCOOT_AUDIO_TEST_DIR/shown"
      echo
    }

    # 22e1. Volume up: the step runs, the level shows (percent form).
    audio_setup 'Volume: 80%' 0 0 "" "" 0 ""
    audio_fifo
    got=$(audio_shown ${audioBehaviorVolume} sink-up)
    [ "$got" = "0:80" ] || { echo "volume up showed '$got', want '0:80' (22e1)" >&2; exit 1; }
    grep -q "set-volume @DEFAULT_AUDIO_SINK@ 5%+" "$SCOOT_AUDIO_TEST_DIR/calls"
    echo "ok: volume up steps and shows the level"

    # 22e2. Fraction form and mute: `1.5 [MUTED]` shows `150 muted`
    #      (past full: wob clamps it into the urgent overflow style).
    audio_setup 'Volume: 1.5 [MUTED]' 0 0 "" "" 0 ""
    audio_fifo
    got=$(audio_shown ${audioBehaviorVolume} sink-mute)
    [ "$got" = "0:150 muted" ] || { echo "mute showed '$got', want '0:150 muted' (22e2)" >&2; exit 1; }
    echo "ok: mute shows the level washed out, past full overflows urgent"

    # 22e3. No device: the step fails -- loud (exit 1), nothing
    #      shown. Both halves fail loud: the step itself...
    audio_setup 'Volume: 80%' 0 1 "" "" 0 ""
    audio_fifo
    if ${audioBehaviorVolume} sink-up 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success on a failed step (22e3)" >&2; exit 1; fi
    # ...and the read-back.
    audio_setup 'Error: no such device' 1 0 "" "" 0 ""
    audio_fifo
    if ${audioBehaviorVolume} sink-up 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success with no device (22e3)" >&2; exit 1; fi
    grep -q "no audio device" "$SCOOT_AUDIO_TEST_DIR/stderr"
    # ...and an unparsable level (no digits at all).
    audio_setup 'Volume: --' 0 0 "" "" 0 ""
    audio_fifo
    if ${audioBehaviorVolume} sink-up 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success on garbage level (21e3)" >&2; exit 1; fi
    grep -q "cannot parse" "$SCOOT_AUDIO_TEST_DIR/stderr"
    echo "ok: no device fails loud per entry"

    # 22e4. Mic mute toggles the source and shows its level.
    audio_setup 'Volume: 0.5' 0 0 "" "" 0 ""
    audio_fifo
    got=$(audio_shown ${audioBehaviorVolume} mic-mute)
    [ "$got" = "0:50" ] || { echo "mic mute showed '$got', want '0:50' (22e4)" >&2; exit 1; }
    grep -q "set-mute @DEFAULT_AUDIO_SOURCE@ toggle" "$SCOOT_AUDIO_TEST_DIR/calls"
    echo "ok: mic mute toggles the source and shows it"

    # 22e5. Brightness: every backlight device steps the same amount
    #      (keyboard LEDs excluded: class `leds`), the OSD shows the
    #      rounded average.
    audio_setup "" 0 0 "$(printf '%b' 'panel,backlight,400,40%,1000\next,backlight,600,60%,1000\nkbd,leds,0,0%,255\n')" "" 0 ""
    audio_fifo
    got=$(audio_shown ${audioBehaviorBrightness} up)
    [ "$got" = "0:50" ] || { echo "brightness showed '$got', want '0:50' (22e5)"; echo "--- devs:"; cat "$SCOOT_AUDIO_TEST_DIR/devs"; echo "--- calls:"; cat "$SCOOT_AUDIO_TEST_DIR/calls"; exit 1; } >&2
    grep -q "brightness-set -e -d panel set +5%" "$SCOOT_AUDIO_TEST_DIR/calls" || { echo "panel step missing (22e5)"; cat "$SCOOT_AUDIO_TEST_DIR/calls"; exit 1; } >&2
    grep -q "brightness-set -e -d ext set +5%" "$SCOOT_AUDIO_TEST_DIR/calls" || { echo "ext step missing (22e5)"; echo "--- devs:"; od -c "$SCOOT_AUDIO_TEST_DIR/devs" | head -8; echo "--- calls:"; cat "$SCOOT_AUDIO_TEST_DIR/calls"; echo "--- probe:"; printf '%b' 'A\neB\n' | od -c | head -3; exit 1; } >&2
    if grep -q "kbd" "$SCOOT_AUDIO_TEST_DIR/calls"; then echo "keyboard LED stepped (22e5)" >&2; exit 1; fi
    echo "ok: brightness steps every panel and shows the average"

    # 22e6. No backlight: loud, naming it.
    audio_setup "" 0 0 "" "" 0 ""
    if ${audioBehaviorBrightness} up 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success with no backlight (22e6)" >&2; exit 1; fi
    grep -q "no backlight devices" "$SCOOT_AUDIO_TEST_DIR/stderr"
    echo "ok: no backlight fails loud per entry"

    # 22e7. Sink list: id plus description, sources excluded.
    audio_setup "" 0 0 "" '[{"id":42,"info":{"props":{"media.class":"Audio/Sink","node.description":"Dummy Output"}}},{"id":43,"info":{"props":{"media.class":"Audio/Sink","node.name":"alsa_out"}}},{"id":99,"info":{"props":{"media.class":"Audio/Source"}}}]' 0 ""
    got=$(${audioBehaviorSink} list)
    want=$(printf '42 Dummy Output\n43 alsa_out')
    [ "$got" = "$want" ] || { echo "sink list showed '$got' (22e7)" >&2; exit 1; }
    echo "ok: sink list names id plus description"

    # 22e8. Sink set by name moves the default and shows its level
    #      (the confirmation on stderr: `list` owns stdout).
    audio_setup 'Volume: 0.5' 0 0 "" '[{"id":42,"info":{"props":{"media.class":"Audio/Sink","node.description":"Dummy Output"}}},{"id":43,"info":{"props":{"media.class":"Audio/Sink","node.name":"alsa_out"}}}]' 0 ""
    audio_fifo
    got=$(audio_shown ${audioBehaviorSink} set alsa_out 2>"$SCOOT_AUDIO_TEST_DIR/stderr")
    [ "$got" = "0:50" ] || { echo "sink set showed '$got', want '0:50' (22e8)" >&2; exit 1; }
    grep -q "Default sink: alsa_out" "$SCOOT_AUDIO_TEST_DIR/stderr"
    grep -q "set-default 43" "$SCOOT_AUDIO_TEST_DIR/calls"
    echo "ok: sink set moves the default and shows it"

    # 22e9. Sink set with no (or an ambiguous) match: loud.
    if ${audioBehaviorSink} set nope 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success on no match (22e9)" >&2; exit 1; fi
    grep -q "no single sink matches" "$SCOOT_AUDIO_TEST_DIR/stderr"
    audio_setup "" 0 0 "" '[{"id":42,"info":{"props":{"media.class":"Audio/Sink","node.description":"Same"}}},{"id":43,"info":{"props":{"media.class":"Audio/Sink","node.description":"Same"}}}]' 0 ""
    if ${audioBehaviorSink} set Same 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success on ambiguity (22e9)" >&2; exit 1; fi
    grep -q "no single sink matches" "$SCOOT_AUDIO_TEST_DIR/stderr"
    echo "ok: sink set refuses no and ambiguous matches"

    # 22f0. Cycle: next after the current default, wrapping past the end.
    audio_setup 'Volume: 0.5' 0 0 "" '[{"id":42,"info":{"props":{"media.class":"Audio/Sink","node.description":"Dummy Output"}}},{"id":43,"info":{"props":{"media.class":"Audio/Sink","node.name":"alsa_out"}}}]' 0 'id 42, type PipeWire:Interface:Node'
    audio_fifo
    audio_shown ${audioBehaviorSink} cycle > /dev/null
    grep -q "set-default 43" "$SCOOT_AUDIO_TEST_DIR/calls"
    printf '%s' 'id 43, type PipeWire:Interface:Node' > "$SCOOT_AUDIO_TEST_DIR/inspect"
    : > "$SCOOT_AUDIO_TEST_DIR/calls"
    audio_fifo
    audio_shown ${audioBehaviorSink} cycle > /dev/null
    grep -q "set-default 42" "$SCOOT_AUDIO_TEST_DIR/calls"
    # ...and with a single sink it stays, saying so.
    audio_setup 'Volume: 0.5' 0 0 "" '[{"id":42,"info":{"props":{"media.class":"Audio/Sink","node.description":"Only"}}}]' 0 'id 42, type PipeWire:Interface:Node'
    audio_fifo
    got=$(audio_shown ${audioBehaviorSink} cycle 2>"$SCOOT_AUDIO_TEST_DIR/stderr")
    grep -q "only one sink, staying" "$SCOOT_AUDIO_TEST_DIR/stderr"
    if grep -q "set-default" "$SCOOT_AUDIO_TEST_DIR/calls"; then echo "cycle moved a single sink (22f0)" >&2; exit 1; fi
    [ "$got" = "0:50" ] || { echo "single-sink cycle showed '$got' (22f0)" >&2; exit 1; }
    echo "ok: sink cycle moves to the next sink and wraps"

    # 22f1. No sinks (or no server): loud.
    audio_setup "" 0 0 "" '[]' 0 ""
    if ${audioBehaviorSink} list 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success with no sinks (22f1)" >&2; exit 1; fi
    grep -q "no audio sinks" "$SCOOT_AUDIO_TEST_DIR/stderr"
    audio_setup "" 0 0 "" "" 1 ""
    if ${audioBehaviorSink} list 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success with no server (22f1)" >&2; exit 1; fi
    grep -q "no audio sinks" "$SCOOT_AUDIO_TEST_DIR/stderr"
    echo "ok: no sinks fails loud per entry"

    # 22f2. `show` validates its percent and lands byte-exact; with no
    #      daemon it fails loud instead of wedging.
    if ${audioBehaviorOsd} show 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success on missing percent (22f2)" >&2; exit 1; fi
    if ${audioBehaviorOsd} show abc 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success on a non-number (22f2)" >&2; exit 1; fi
    if ${audioBehaviorOsd} frobnicate 2>/dev/null; then echo "silent success on a bogus command (22f2)" >&2; exit 1; fi
    audio_fifo
    got=$(audio_shown ${audioBehaviorOsd} show 42 muted)
    [ "$got" = "0:42 muted" ] || { echo "show landed '$got', want '0:42 muted' (22f2)" >&2; exit 1; }
    export XDG_RUNTIME_DIR="$SCOOT_AUDIO_TEST_DIR/no-rt"
    mkdir -p "$XDG_RUNTIME_DIR"
    if ${audioBehaviorOsd} show 42 2>"$SCOOT_AUDIO_TEST_DIR/stderr"; then echo "silent success with no daemon (22f2)" >&2; exit 1; fi
    grep -q "no OSD running" "$SCOOT_AUDIO_TEST_DIR/stderr"
    echo "ok: show validates, lands byte-exact, and misses loud with no daemon"
  ''}

  touch $out
  echo "scoot-modules: all file-content checks passed"
''
