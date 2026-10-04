{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;

  # The login-screen entry. Built as a package exposing
  # `share/wayland-sessions/scoot.desktop` with `providedSessions`,
  # which is the shape `services.displayManager.sessionPackages`
  # requires (see the `addCheck` on that option in nixpkgs'
  # `nixos/modules/services/display-managers/default.nix` at the
  # pinned rev: an element without `providedSessions` is rejected at
  # eval). Consumed via XDG_DATA_DIRS by greetd/tuigreet, GDM, SDDM --
  # whichever reads wayland-sessions -- with no change to the default
  # session: this module never sets `services.displayManager.defaultSession`
  # (pre-select) or `services.greetd.settings.default_session` /
  # `initial_session` (what actually runs), so enabling it adds scoot
  # to the menu next to existing sessions and switches nothing.
  #
  # Auto-login caveat, verified against the same file: with no explicit
  # `defaultSession`, `autologinSession` falls back to the head of the
  # session list, so on a box that auto-logs-in, adding ANY session
  # package can move the autologin target. If you use autoLogin, pin
  # `services.displayManager.defaultSession` explicitly.
  #
  # The entry runs `scoot-session`, not `scoot --tty` directly: the
  # launcher (`resources/scoot-session`, shipped beside the binary)
  # imports the login environment into the user manager, starts
  # `scoot.service`, waits for IPC readiness, imports `WAYLAND_DISPLAY`
  # and `XDG_CURRENT_DESKTOP` into the manager and the D-Bus activation
  # environment, and on exit stops the session target so session-bound
  # units stop. See docs/nix.md for the whole flow.
  sessionPackage =
    (pkgs.writeTextDir "share/wayland-sessions/scoot.desktop" ''
      [Desktop Entry]
      Name=scoot
      Comment=scoot scrolling-tiling Wayland compositor
      Exec=${
        if cfg.session.command == null then "${cfg.package}/bin/scoot-session" else cfg.session.command
      }
      Type=Application
      DesktopNames=scoot
    '').overrideAttrs
      (old: {
        # `writeTextDir` names the derivation after no real package;
        # give the session entry a stable name for the store path.
        name = "scoot-wayland-session";
        passthru = (old.passthru or { }) // {
          providedSessions = [ "scoot" ];
        };
      });

  # Whether Stylix's own regreet target would set ReGreet's background
  # (see the `background` assertion below): Stylix present, with an
  # image, and its regreet target enabled. Written defensively because
  # Stylix is an optional import that may be absent entirely (then
  # `stylix` is `{}`) or predate the regreet target (then `.regreet` is
  # `{}`) -- every level defaults to off.
  stylixRegreetBackground =
    stylix:
    (stylix.image or null) != null && (((stylix.targets or { }).regreet or { }).enable or false);

  # The user units behind `session.enable`, single-sourced from
  # `resources/systemd/user/`: `@SCOOT_BIN@` becomes this package's
  # binary, so the entry above and the service always launch the same
  # build (including the `scoot-xwayland` wrapper's `PATH` append, when
  # that is the package). Read at eval, like the desktop file: no
  # package rebuild when only the units change.
  sessionUnits = {
    "scoot.service".text = builtins.replaceStrings [ "@SCOOT_BIN@" ] [ "${cfg.package}/bin/scoot" ] (
      builtins.readFile ../../resources/systemd/user/scoot.service
    );
    # The shutdown target names no binary: no substitution to keep in
    # step, plain text.
    "scoot-shutdown.target".text = builtins.readFile ../../resources/systemd/user/scoot-shutdown.target;
  };
in
{
  # nixpkgs' own ReGreet module, so `services.displayManager.regreet`
  # exists wherever this module is used -- including standalone
  # option-only evaluations that never import a full NixOS (like
  # `nix/tests.nix`): without it, the `greeter.enable` wiring below
  # would set an undeclared option. Already imported on real NixOS (it
  # is a default module there), where a repeated import is a no-op. The
  # path is pinned-rev: a consumer's nixpkgs predating
  # `services/display-managers/regreet.nix` fails the import loudly,
  # naming the file.
  imports = [ "${pkgs.path}/nixos/modules/services/display-managers/regreet.nix" ];

  options.programs.scoot = {
    enable = lib.mkEnableOption "scoot, the scrolling-tiling Wayland compositor";

    # `pkgs.scoot` when the flake's overlay (`overlays.default`) is
    # applied, else null: nothing is guessed, since a `scoot` from anywhere
    # else would silently install someone else's build. The flake wrapper
    # (`nixosModules.scoot` in `flake.nix`) fills this with the flake's own
    # build via `mkDefault` either way.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scoot or null;
      defaultText = lib.literalExpression "pkgs.scoot or null";
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.system}.scoot";
      description = ''
        The scoot package to install system-wide and to launch from
        the login-screen session entry.
      '';
    };

    # scootbg, the wallpaper daemon a `[wallpaper]` config section runs.
    # The config file itself is per-user (the home-manager module); this
    # only guarantees the binary, on the system PATH, where the default
    # `[wallpaper] command = "scootbg"` finds it -- for any user, and for a
    # session the greeter starts.
    wallpaper = {
      # On whenever scoot is and there is a scootbg to install: a ~1.7 MB
      # binary that does nothing until a config asks for a wallpaper, so
      # installing it changes nothing -- unlike the login entry, which
      # stays opt-in. With no package (a direct-module user without the
      # flake's overlay) it stays off rather than failing evaluation, so
      # upgrading breaks nobody; `true` set explicitly with no package is
      # the loud assertion below. `false` opts out (another wallpaper
      # daemon, say).
      enable = lib.mkOption {
        type = lib.types.bool;
        default = cfg.enable && cfg.wallpaper.package != null;
        defaultText = lib.literalExpression "config.programs.scoot.enable && config.programs.scoot.wallpaper.package != null";
        description = ''
          Install `programs.scoot.wallpaper.package` system-wide, so a
          `[wallpaper]` section in a user's scoot config finds `scootbg`
          on PATH. On by default whenever `programs.scoot.enable` is and a
          package is available (the flake's modules and overlay provide
          one); setting it to `true` with no package fails evaluation.
        '';
      };

      # `pkgs.scootbg` with the flake's overlay, else null; the flake
      # wrapper injects the flake's own build (the same revision as
      # `package`, so the `apply-config` scoot speaks is the one scootbg
      # understands). A direct-module user who enables this with neither
      # gets the assertion below at eval, not a session with no wallpaper.
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = pkgs.scootbg or null;
        defaultText = lib.literalExpression "pkgs.scootbg or null";
        example = lib.literalExpression "inputs.scoot.packages.\${pkgs.system}.scootbg";
        description = ''
          The scootbg package to install when `wallpaper.enable` is set.
        '';
      };
    };

    session = {
      # Default OFF, explicitly. Adding the entry is safe (it never
      # replaces the default session -- see above), but a login-screen
      # change is still a change to the user's way back into their own
      # desktop, and per CLAUDE.md's never-strand rule that stays an
      # explicit opt-in, not something `enable = true` smuggles in.
      # `enable` alone installs the binary; the session entry is separate.
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Add a scoot entry to the display-manager/greetd session menu,
          alongside existing sessions. Never replaces the default
          session. If you use autoLogin, also pin
          `services.displayManager.defaultSession` explicitly (see the
          autologin caveat in the module source).
        '';
      };
      # Full `Exec=` line for the session entry, NOT just a suffix
      # appended after `--` -- deliberately. The issue-#171 acceptance
      # shape is a wrapper script (`Exec=<wrapper>/bin/scoot-session`,
      # which itself runs `scoot --tty -- ...` plus stderr to a log),
      # and a plain `-- COMMAND` append cannot express a wrapper (it
      # would nest scoot inside scoot). A verbatim string expresses all
      # three shapes: the launcher default (null), the common append (write
      # the full `scoot --tty -- ...` line, e.g. pointing at the
      # home-manager module's `sessionScript` output), and a wrapper
      # path. It is a string rather than an argv list for the same
      # reason: an `Exec=` line is a command line, and auto-escaping
      # would fight wrapper paths and per the Desktop Entry Spec the
      # quoting is the author's anyway. The cost is stated in the option
      # description itself (not just here): a
      # set value replaces the whole line, so dropping `--tty` (or the
      # binary path) breaks the entry loudly at the greeter, not at
      # eval -- copy the example shape. The entry stays additive and
      # default-off whatever the value (see above), so a broken line
      # strands nobody: the other sessions remain.
      #
      # A set value bypasses `scoot-session`: only the null default runs
      # the launcher, so only it gets the session wiring (user-manager
      # import, `graphical-session.target`, the activation environment,
      # teardown). A `scoot --tty -- <script>` value still runs exactly
      # what it says -- a session, just an unwired one -- and startup
      # programs that want the wiring belong in scoot's `[autostart]`,
      # which runs inside it (see docs/nix.md).
      command = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = lib.literalExpression ''
          "''${config.programs.scoot.package}/bin/scoot --tty -- /home/alice/.config/scoot/session.sh"
        '';
        description = ''
          Full `Exec=` command line for the login-screen session entry.
          Null (the default) renders `<package>/bin/scoot-session`, the
          session launcher, so existing configs gain the session wiring
          (user-manager import, `graphical-session.target`, the D-Bus
          activation environment, teardown on exit -- see docs/nix.md).
          Set it to run something else instead: the usual shape is
          `<package>/bin/scoot --tty -- <command>`, e.g. the
          home-manager module's `sessionScript` output at
          `/home/alice/.config/scoot/session.sh` (for a user `alice`;
          see `programs.scoot.sessionScript` and docs/nix.md, which show
          the pairing together). A wrapper script path (logging,
          environment setup) works too -- that is the gh-issue-#171
          acceptance shape. A set value replaces the whole line and
          bypasses the launcher, so a set entry runs without the session
          wiring (for wired startup programs, use `[autostart]`
          instead); dropping `--tty` (or the binary path) breaks the
          entry loudly at the greeter, not at eval -- copy the example
          shape. `Exec=` lines get no shell expansion (`~` and `$HOME`
          arrive literally), so always use an absolute path.
        '';
      };
    };

    greeter = {
      # Default OFF, explicitly, and more strongly than `session.enable`:
      # this replaces the login screen (greetd running ReGreet under
      # cage, via nixpkgs' own `services.displayManager.regreet`), so it
      # is only ever on when the user sets it. Roll back by turning it
      # off again (or by booting the previous generation): nothing about
      # the previous login screen is uninstalled while it is on, only
      # displaced. See docs/nix.md.
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Log in through ReGreet (greetd + cage, nixpkgs' own
          `services.displayManager.regreet`), with scoot in its session
          list. Only ever on when set: this replaces the login screen.
          Conflicts with GDM and SDDM (refused at eval); anything else
          that owns the login screen must be turned off by hand.
        '';
      };

      # ReGreet's backdrop, when the admin wants it to match the session.
      # Null (the default) leaves ReGreet's background alone -- which is
      # also what to do under Stylix with its regreet target enabled:
      # Stylix then sets the background from `stylix.image` itself, and
      # setting both is refused at eval. Otherwise, point this at an
      # image (it is copied to the store) -- usually the same file as
      # the session wallpaper -- and it becomes ReGreet's
      # `background.path`. No `fit` knob here: ReGreet's default stands
      # unless set through `services.displayManager.regreet.settings`
      # directly (which also wins over this path: leave it null then).
      background = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        example = "/home/alice/Pictures/hills.jpg";
        description = ''
          Background image for the ReGreet login screen
          (`background.path` in its config). Null leaves it alone (and
          is required under Stylix with its regreet target, which sets
          the background from `stylix.image` itself).
        '';
      };
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.wallpaper.enable {
      assertions = [
        {
          # Loud at eval, not a `[wallpaper]` that logs "scootbg was not
          # found" at every login. Reachable only by setting `enable = true`
          # explicitly: the default is off without a package.
          assertion = cfg.wallpaper.package != null;
          message = ''
            programs.scoot.wallpaper.enable is set to true but
            programs.scoot.wallpaper.package is null. Flake consumers get
            the flake's own scootbg through the nixosModules wrapper, and
            the flake's overlay provides pkgs.scootbg -- this fires only
            for direct-module use without either: set
            programs.scoot.wallpaper.package, apply the overlay, or leave
            programs.scoot.wallpaper.enable at its default.
          '';
        }
      ];
      # The `!= null` guard keeps a null out of the list, so the failure is
      # the assertion's message rather than a type error.
      environment.systemPackages = lib.optional (cfg.wallpaper.package != null) cfg.wallpaper.package;
    })
    (lib.mkIf cfg.enable {
      assertions = [
        {
          # Loud at eval, not a broken `.desktop` at login: `Exec=` with
          # a null package would interpolate to nothing and greet
          # whoever picks the entry with a greeter error.
          assertion = cfg.package != null;
          message = ''
            programs.scoot.enable is set but programs.scoot.package is null.
            Flake consumers get the flake's own build by default through the
            nixosModules wrapper, and the flake's overlay provides pkgs.scoot
            -- this fires only for direct-module use without either: set
            programs.scoot.package explicitly.
          '';
        }
        {
          assertion = cfg.session.enable -> cfg.package != null;
          message = ''
            programs.scoot.session.enable needs programs.scoot.package:
            a session entry with no binary to launch would fail at the login screen.
          '';
        }
        {
          # Loud at eval, not an empty `Exec=` at login: an explicitly
          # empty or whitespace-only command would render a .desktop with
          # nothing (or blanks) to launch. (`builtins.match` returns null
          # on no match, so the second disjunct is false exactly for
          # empty/blank strings.)
          # (The default null -- the launcher entry -- never reaches here.)
          assertion =
            cfg.session.command == null || builtins.match "^[[:space:]]*$" cfg.session.command == null;
          message = ''
            programs.scoot.session.command is set but empty or blank:
            either leave it null for the default
            `<package>/bin/scoot-session` launcher entry, or set the full
            Exec= command line to run.
          '';
        }
      ];

      environment.systemPackages = lib.optional (cfg.package != null) cfg.package;

      # The `cfg.package != null` conjunct is load-bearing, not redundant
      # with the assertions above: `sessionPackage` interpolates
      # `cfg.package` into `Exec=`, and a null there fails while
      # evaluating the config value itself -- before `assertions` are
      # checked -- with a bare "cannot coerce null to a string". The
      # conjunct keeps the null out of the interpolation so the failure
      # surfaces as the helpful assertion message instead.
      services.displayManager.sessionPackages = lib.optional (
        cfg.session.enable && cfg.package != null
      ) sessionPackage;

      # The launcher's units, beside the entry that starts them. Same
      # gate and same null guard as the entry: no entry, no units, and
      # never a unit naming a binary that is not there.
      systemd.user.units = lib.mkIf (cfg.session.enable && cfg.package != null) sessionUnits;
    })
    # The login screen, in its own element (not under `cfg.enable`):
    # with `enable` off and `greeter.enable` on, the assertions below
    # must still fire rather than the whole element going quiet.
    (lib.mkIf cfg.greeter.enable {
      assertions = [
        {
          # A greeter for a compositor that is not installed is
          # nonsense; without `enable` there is no package for the
          # greeter's session list to offer.
          assertion = cfg.enable;
          message = ''
            programs.scoot.greeter.enable needs programs.scoot.enable:
            the greeter lists scoot sessions, so scoot itself must be installed.
          '';
        }
        {
          # The greeter lists scoot through the session entry (ReGreet
          # reads `wayland-sessions`, which is what `session.enable`
          # installs into). Forced on below by default, so this fires
          # only when the entry was explicitly turned back off.
          assertion = cfg.session.enable;
          message = ''
            programs.scoot.greeter.enable needs
            programs.scoot.session.enable: without the session entry the
            greeter has no scoot session to offer.
          '';
        }
        {
          # The login screen is single-owner: greetd/ReGreet and GDM
          # cannot both run it. Loud at eval, not two greeters fighting
          # over the first VT at boot.
          assertion = !config.services.displayManager.gdm.enable;
          message = ''
            programs.scoot.greeter.enable conflicts with GDM
            (services.displayManager.gdm.enable): turn one of them off.
            Rolling back is the previous NixOS generation, or this
            option set back to false.
          '';
        }
        {
          # Same, for SDDM. Anything else owning the login screen
          # (lemurs, ly, another greetd setup) must be turned off by
          # hand -- see docs/nix.md.
          assertion = !config.services.displayManager.sddm.enable;
          message = ''
            programs.scoot.greeter.enable conflicts with SDDM
            (services.displayManager.sddm.enable): turn one of them off.
            Rolling back is the previous NixOS generation, or this
            option set back to false.
          '';
        }
        {
          # Two owners for one backdrop: an explicit `background` here
          # and Stylix's regreet target (which sets the same
          # `background.path` from `stylix.image`) would merge-conflict.
          # `config.stylix` is unset without Stylix, and `.regreet`
          # without that target -- both default to off (see the helper
          # above).
          assertion = cfg.greeter.background == null || !(stylixRegreetBackground (config.stylix or { }));
          message = ''
            programs.scoot.greeter.background is set, but Stylix's
            regreet target is also setting ReGreet's background from
            `stylix.image`: unset one of them (leave `background` null
            and Stylix owns the backdrop, or turn off
            `stylix.targets.regreet` image theming).
          '';
        }
      ];

      # `session.enable` on by default, so the greeter lists a scoot
      # session; an explicit `false` there trips the assertion above
      # instead of yielding a greeter with no scoot in it.
      programs.scoot.session.enable = lib.mkDefault true;

      # The login screen itself is nixpkgs' own ReGreet module (greetd
      # running ReGreet under cage) -- this only turns it on and
      # optionally names its backdrop at plain priority, so a backdrop
      # set directly under `services.displayManager.regreet.settings`
      # behaves like any other explicit-against-explicit conflict
      # instead of being silently shadowed. ReGreet runs under cage,
      # never inside scoot: hosting a pre-login greeter in the
      # compositor would need a locked-down scoot profile (no binds, no
      # IPC socket), which is out of scope -- see docs/nix.md.
      services.displayManager.regreet = {
        enable = true;
        # The whole table or nothing: a `path`-only `mkIf` would leave
        # an empty `[background]` behind when `background` is null.
        settings.background = lib.mkIf (cfg.greeter.background != null) {
          path = toString cfg.greeter.background;
        };
      };
    })
  ];
}
