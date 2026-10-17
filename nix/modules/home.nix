{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  tomlFormat = pkgs.formats.toml { };

  # The desktop profile's shared option subtree.
  desktop = import ./desktop.nix { inherit lib; };

  # The Nix side of the look catalog (sources, build-time renders): names
  # of shipped looks or paths to source look directories, read with
  # `builtins.fromTOML` (no derivations, no import-from-derivation). Null
  # `lookPackage` keeps the whole catalog side off (see the option).
  lookCatalogLib = import ../look-catalog.nix {
    inherit lib pkgs;
    scootLook = cfg.lookPackage;
    fallbackUiPackage = config.programs.scoot.desktop.theme.fonts.uiPackage or null;
    fallbackFont =
      if pkgs.stdenv.hostPlatform.isLinux then
        "${pkgs.dejavu_fonts.minimal}/share/fonts/truetype/DejaVuSans.ttf"
      else
        null;
  };

  # Stylix is not an input of this flake: its presence is
  # `config.lib.stylix` (defined by its palette module whether or not it is
  # enabled), and `stylix.enable` (false by default in Stylix itself) says
  # whether it is asked to theme anything. Same detection as
  # `nix/modules/scootbar.nix`.
  stylixPresent = config.lib ? stylix && (config.stylix.enable or false);

  # Stylix synthesis (the `stylix` look): on when scoot is managed, its
  # switch is on, Stylix itself is in use, and a render binary is on
  # hand. The look carries the palette; the declared default below
  # follows it.
  stylixActive = cfg.enable && cfg.stylix.enable && stylixPresent && cfg.lookPackage != null;

  # The resolved polarity for the synthesis: `light`/`dark` verbatim,
  # `either` (Stylix's default: automatic) by the luma rule (exact
  # integer math over `base00`/`base05`; the Nix side always passes
  # polarity explicitly, so the static files and the rendered look
  # agree by construction).
  stylixLib = import ../lib/look-from-stylix.nix { inherit lib; };
  stylixDark =
    if !stylixActive then
      false
    else if (config.stylix.polarity or "either") == "dark" then
      true
    else if (config.stylix.polarity or "either") == "light" then
      false
    else
      stylixLib.isDarkByLuma config.lib.stylix.colors.withHashtag.base00
        config.lib.stylix.colors.withHashtag.base05;

  lookCatalog = lookCatalogLib.mkCatalog {
    looks = cfg.looks;
    stylix =
      if !stylixActive then
        null
      else
        {
          extends = cfg.stylix.extends;
          colors = config.lib.stylix.colors.withHashtag;
          fontsSans = config.stylix.fonts.sansSerif or null;
          fontsMono = config.stylix.fonts.monospace or null;
          sizes = {
            desktop = (config.stylix.fonts.sizes or { }).desktop or null;
            popups = (config.stylix.fonts.sizes or { }).popups or null;
            applications = (config.stylix.fonts.sizes or { }).applications or null;
            terminal = (config.stylix.fonts.sizes or { }).terminal or null;
          };
          opacity = {
            popups = (config.stylix.opacity or { }).popups or null;
            terminal = (config.stylix.opacity or { }).terminal or null;
          };
          cursor = config.stylix.cursor or null;
          # Gated by `stylix.wallpaper.enable` (on by default): off
          # leaves the image out, so a `color` set beside it stands
          # alone instead of pairing an image with a color.
          image =
            if cfg.stylix.wallpaper.enable then (config.stylix.image or null) else null;
          imageMode = config.stylix.imageScalingMode or "fill";
          isDark = stylixDark;
        };
  };

  # Rendered from the user's free-form settings. A value with no TOML
  # representation at all (e.g. a function) fails the option type-check
  # at evaluation time ("not of type 'TOML value'"), so the error aborts
  # before anything builds, let alone starts a session. A value that renders
  # but has the wrong *scoot* type (a string for `layout.gap`) reaches
  # the session and is refused there -- and the loader fails safe (whole
  # file discarded for defaults, session still boots), so a typo costs
  # the config, never the session. See
  # site/src/content/docs/scoot/configure.md#failure-semantics.
  configFile = tomlFormat.generate "scoot-config.toml" renderedSettings;

  # `settings`, plus `wallpaper.command` pointing at the installed scootbg
  # when the settings have a `[wallpaper]` table and the module installs
  # the binary -- so the section works whatever is on PATH. A `command`
  # the user set wins. Only for a table: anything else under `wallpaper`
  # renders as written and is refused, by name, by scoot itself. The store
  # path changes with every upgrade; scootbg leaves `command` out of the
  # section's fingerprint, so an upgrade never re-applies the section over
  # a `scootbg set` pick.
  hasWallpaper = cfg.settings ? wallpaper;
  injectCommand =
    cfg.wallpaper.enable
    && cfg.wallpaper.package != null
    && hasWallpaper
    && builtins.isAttrs cfg.settings.wallpaper;

  # An `image` as a look or a user gives it: a path renders as its store
  # path; `{ url, hash }` is fetched once, into the store, at build time
  # (`pkgs.fetchurl`, hash verified by Nix), so scootbg and the rendered
  # TOML only ever see a file. Anything else shaped as a set is a loud
  # eval error, not a TOML table scoot would refuse at startup.
  resolveWallpaperImage =
    image:
    if builtins.isAttrs image then
      if image ? url && image ? hash && builtins.isString image.url && builtins.isString image.hash then
        pkgs.fetchurl {
          url = image.url;
          sha256 = image.hash;
        }
      else
        throw "programs.scoot wallpaper `image` as a set needs string `url` and `hash` (hash as SRI (`sha256-...`) or hex)"
    else
      image;

  # One `[wallpaper]` table with every `{ url, hash }` image — the
  # top-level `image` and each per-output `output.<name>.image` —
  # fetched into the store. Anything else renders as written: left as
  # a set, one would render as a TOML table that scoot's strict section
  # parse refuses at startup, so a set without string `url` and `hash`
  # fails here instead, naming the missing half.
  resolveWallpaperTable =
    table:
    let
      top =
        if builtins.isAttrs (table.image or null) then
          table // { image = toString (resolveWallpaperImage table.image); }
        else
          table;
    in
    if builtins.isAttrs (top.output or null) then
      top
      // {
        output = builtins.mapAttrs (
          name: per:
          if builtins.isAttrs per && builtins.isAttrs (per.image or null) then
            per // { image = toString (resolveWallpaperImage per.image); }
          else
            per
        ) top.output;
      }
    else
      top;

  renderedSettings =
    let
      commanded =
        if injectCommand then
          cfg.settings
          // {
            wallpaper = {
              command = lib.getExe' cfg.wallpaper.package "scootbg";
            }
            // cfg.settings.wallpaper;
          }
        else
          cfg.settings;
      # A user's `{ url, hash }` images, fetched like a look's above:
      # the TOML only takes strings here, so each derivation
      # stringifies to its output path. Anything else under `wallpaper`
      # (a color, a plain string image, a non-table) renders as written.
      fetched =
        if commanded ? wallpaper && builtins.isAttrs commanded.wallpaper then
          commanded // { wallpaper = resolveWallpaperTable commanded.wallpaper; }
        else
          commanded;
    in
    fetched;

  # Session script path: beside the rendered config, derived from
  # `configFile`'s directory, so a relocated config keeps its script
  # next to it (`scoot/session.sh` by default -- `dirOf` answers
  # `"scoot"`, i.e. the default path is unchanged). A bare filename
  # (`configFile = "config.toml"`) has no directory (`dirOf` answers
  # `"."`); the script then lands at the config root as `session.sh`.
  # `configFile` is relative to $XDG_CONFIG_HOME by contract (see the
  # option), so an absolute path is already user error on the config
  # half and stays one here.
  scriptPath =
    let
      d = builtins.dirOf cfg.configFile;
    in
    if d == "." || d == "" then "session.sh" else "${d}/session.sh";
in
{
  # The idle policy's user half (swayidle unit, inhibitor, locker
  # config) and the notification daemon's user half (mako unit, its
  # config, the bar feed): options under `programs.scoot.desktop.idle`
  # and `...notifications`, declared there. Imported here (not by
  # consumers) so the profile's user side is one import. The audio
  # slot's user half (the OSD unit, its config, the control scripts),
  # the privilege-prompt and keyring halves (the agent package, the
  # secrets client), the capture slot's user half, the apps slot's
  # user half (the terminal and file manager, the `xdg-open`
  # plumbing, the WiFi/Bluetooth pickers, the automounter), and the
  # theme's
  # user half (fonts, cursor, GTK/Qt settings, the dark-mode signal,
  # the look's app files) ride the same way.
  #
  # The shared keymap's user half (`programs.scoot.desktop.keys`):
  # the `[binds]`, its tools and its slot scripts. Same pattern.
  imports = [
    ./idle-home.nix
    ./notifications-home.nix
    ./clipboard-home.nix
    ./launcher-home.nix
    ./capture-home.nix
    ./auth-home.nix
    ./audio-home.nix
    ./nightlight-home.nix
    ./power-home.nix
    ./displays-home.nix
    ./apps-home.nix
    ./inputmethod-home.nix
    ./keys-home.nix
    ./theme-home.nix
  ];

  options.programs.scoot = {
    enable = lib.mkEnableOption "scoot, the scrolling-tiling Wayland compositor";

    # One switch plus a look choice for a working desktop (see
    # `desktop.nix` and site/src/content/docs/desktop/index.md). Each side wires only what it owns;
    # this side owns the config file (the look's `[appearance]` and
    # `[wallpaper]`), the portal config and the `[xwayland]` knob.
    desktop = desktop.options;

    # The look catalog: names of shipped looks (`ginger-night`,
    # `moonrise`, `music-desk`, `radial-burst`, `vinyl-sunset`) or paths
    # to source look directories. Each entry renders at build time
    # through the `scoot-look` binary and links as
    # `~/.local/share/scoot/looks/<name>` (sources) and
    # `~/.local/share/scoot/rendered/<name>` (renders), in list order —
    # first wins, so a path named like a shipped look shadows it. A
    # shipped name outside the five, a directory without a readable
    # `look.toml`, or a manifest whose `name` is not a look name fails
    # the evaluation naming it. Read only with `desktop.enable`. A
    # literal `stylix` means the synthesized Stylix look (below): it
    # appends when synthesis is on, and a directory named `stylix`
    # goes as a path, not a string.
    looks = lib.mkOption {
      type = lib.types.listOf (lib.types.either lib.types.str lib.types.path);
      default = builtins.attrNames (import ./desktop.nix { inherit lib; }).looks;
      defaultText = lib.literalExpression "every shipped look";
      example = [
        "ginger-night"
        "moonrise"
        ./my-looks/moonrise-green
      ];
      description = ''
        The looks home-manager installs: shipped look names or paths to
        source look directories, plus `stylix` when Stylix synthesis is
        on. Default: every shipped look. A look with `extends`
        resolves its base through this same list, so list the base too.
      '';
    };

    # The declared default: what a fresh state starts on. Null removes
    # the look layer. The live choice is state home-manager does not
    # own: each activation runs `scoot look sync`, which starts on this
    # when nothing is recorded, switches to it when it changed, falls
    # back to it when the live choice left the catalog, and otherwise
    # keeps the live choice. With the profile on and no choice, the
    # look is `ginger-night` (the profile sets it below). An unknown
    # name fails the evaluation naming the catalog.
    look = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      defaultText = lib.literalMD ''
        `"ginger-night"` when `desktop.enable` is on (the profile sets it),
        `null` (nothing themed) otherwise
      '';
      example = "moonrise";
      description = ''
        One name from `looks`: the declared default look. Moved here
        from `programs.scoot.desktop.look` (breaking): update the path.
        Setting `look = null` explicitly is the opt-out, and themes
        nothing. Each value is a default a value you set in `settings`
        beats per key; the live choice in `<state>/look.json` beats this
        until it changes (see `scoot look sync`).
      '';
    };

    # The `scoot-look` binary that renders the catalog at build time:
    # `pkgs.scoot-look` with the flake's overlay, else null. Null keeps
    # the whole catalog side off (no sources, no renders, no declared
    # default, no activation step): a files-only setup manages no looks.
    lookPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scoot-look or null;
      defaultText = lib.literalExpression "pkgs.scoot-look or null";
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.stdenv.hostPlatform.system}.scoot-look";
      description = ''
        The scoot-look package that renders `looks` at build time. Null
        installs no catalog: `look sync` then has no declared default
        and leaves the live choice alone.
      '';
    };

    # `pkgs.scoot` when the flake's overlay (`overlays.default`) is
    # applied, else null: nothing is guessed, since a `scoot` from anywhere
    # else would silently install someone else's build. The flake wrapper
    # (`homeModules.scoot` in `flake.nix`) fills this with the flake's own build via `mkDefault` on
    # Linux and with null (files only) on Darwin; direct-module users
    # without the overlay set it explicitly (see the `example`), or leave
    # it null for a files-only setup -- the module manages files
    # regardless, and asserts nothing about the package.
    package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = pkgs.scoot or null;
      defaultText = lib.literalExpression "pkgs.scoot or null";
      example = lib.literalExpression "inputs.scoot.packages.\${pkgs.stdenv.hostPlatform.system}.scoot";
      description = ''
        The scoot package to install. Null means install no binary --
        the module then only manages files.
      '';
    };

    # Free-form TOML value, NOT a typed schema per field -- deliberately.
    # The config moves fast (three tables landed this month); a typed
    # schema goes stale and then lies about what the compositor accepts,
    # while free-form never drifts. `[binds]` is arbitrary keys anyway,
    # so a schema would need an escape hatch exactly where most of the
    # user content lives. Typed options exist only where they carry
    # behavior beyond the file (`enable`, `package`, `configFile`,
    # `sessionScript`, `portals.enable`); everything the compositor
    # reads passes through here verbatim. Field names, types and
    # defaults are documented in site/src/content/docs/scoot/configure.md, and the worked
    # example in site/src/content/docs/scoot/configure.md#example-configtoml is pasted from a live
    # `scoot --print-default-config` emission, not hand-written.
    settings = lib.mkOption {
      type = tomlFormat.type;
      default = { };
      example = {
        layout.gap = 8;
        binds = {
          "super+t" = "spawn foot";
          "ctrl+alt+space" = "spawn fuzzel";
        };
        autostart.commands = [ "spawn waybar" ];
      };
      description = ''
        Free-form scoot configuration, rendered verbatim to TOML.
        An empty set renders a valid minimal file (the compositor runs
        on built-in defaults, exactly as with no file at all).
      '';
    };

    # Stylix as a source of a look (design section 5): on by default,
    # effective only when Stylix itself is in use (see `stylixPresent`
    # above) and a render binary is on hand. On, the catalog gains a
    # synthesized look named `stylix` (the palette, fonts, sizes,
    # opacity, cursor and image serialized through `scoot-look
    # import-base16`) and the declared default below follows it. No app
    # module reads Stylix: it reaches the desktop only as that look.
    # Nothing here needs Stylix, and nothing changes without it.
    stylix = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Synthesize the `stylix` look from Stylix when it is in use
          (`config.lib.stylix` exists and `stylix.enable` is on) and
          make it the declared default. A value you set in `settings`
          always wins over the look; this only turns the synthesis off.
          Nothing here needs Stylix, and nothing changes without it.
        '';
      };

      # The base the synthesized look extends: what fills every token
      # the palette does not set (the wallpaper when `stylix.image` is
      # unset, the terminal hues and static editor leaves, the night
      # temperature, shape and opacity). Default: `ginger-night` with
      # the desktop profile on (closest to what profile users see
      # today), none otherwise. The base resolves through `looks`, so
      # list it there; a custom path leaves the eval-time static
      # leaves inert (the runtime layer still works).
      extends = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = if (cfg.desktop.enable or false) then "ginger-night" else null;
        defaultText = lib.literalExpression ''
          "ginger-night" when programs.scoot.desktop.enable is on, null otherwise
        '';
        example = "moonrise";
        description = ''
          The look the synthesized `stylix` look extends. Null extends
          nothing: a standalone look of the Stylix palette alone.
        '';
      };

      # Just the Stylix wallpaper defaults, so a solid-color background
      # stays reachable under Stylix: set this to `false` to leave
      # `stylix.image` out of the synthesized look and choose your own
      # wallpaper color (a `color` you set stands alone) or image. The
      # palette, fonts, cursor and the rest stay Stylix's.
      wallpaper.enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Carry `stylix.image` and `stylix.imageScalingMode` into the
          synthesized `stylix` look when theming is on. Set to `false`
          to leave the image out: the look inherits its wallpaper from
          `extends` (or carries a flat background color without one),
          so a `color` you set stands alone.
        '';
      };

      # The registry-shaped theme value for `stylix` (the Stylix
      # palette over the base look's static leaves): what
      # `theme-look.nix` resolves the `stylix` look to. Internal: set
      # beside the catalog, read by the theme.
      theme = lib.mkOption {
        type = lib.types.nullOr lib.types.attrs;
        internal = true;
        default = lookCatalog.stylixTheme;
        description = ''
          The synthesized theme value for the `stylix` look.
        '';
      };
    };

    # Where the rendered TOML lands, relative to $XDG_CONFIG_HOME
    # (i.e. `xdg.configFile`). Overriding this is the only way to move
    # the config: scoot itself reads a fixed default path unless passed
    # `--config`, and the module does not launch scoot, so a path here
    # that scoot never reads would be a file to nowhere.
    configFile = lib.mkOption {
      type = lib.types.str;
      default = "scoot/config.toml";
      example = "scoot/config.toml";
      description = ''
        Location of the rendered config, relative to $XDG_CONFIG_HOME.
        Keep the default unless you also pass the same path via
        `--config` wherever you launch scoot.
      '';
    };

    # Behavior half of "Starting a session" (see
    # site/src/content/docs/scoot/index.md#starting-a-session): the config declares the
    # baseline, a script carries ordering/conditionals. When set, this
    # text is written executable beside the config (at
    # `<dirOf configFile>/session.sh` -- `scoot/session.sh` by default);
    # launch it with `scoot -- ~/.config/<that path>` (or exec it from
    # your greetd/startwm entry). On NixOS, the matching half is the
    # NixOS module's `programs.scoot.session.command`: set it to the full
    # `<package>/bin/scoot --tty -- /home/<user>/.config/<that path>`
    # line -- an absolute path, since `Exec=` lines get no shell
    # expansion -- so the greeter entry runs this script instead of the
    # `scoot-session` launcher (see site/src/content/docs/desktop/index.md#the-greeter, which shows the pairing
    # together). That entry runs without the launcher's session wiring;
    # startup programs that want the wiring belong in `[autostart]`
    # instead. Null writes no file.
    sessionScript = lib.mkOption {
      type = lib.types.nullOr lib.types.lines;
      default = null;
      example = ''
        waybar &
        mako &
        exec foot
      '';
      description = ''
        Session startup script, written executable beside the rendered
        config at `<dirOf configFile>/session.sh` (`scoot/session.sh`
        with the default `configFile`). Launch it with
        `scoot -- ~/.config/scoot/session.sh` for the default (or
        `~/.config/<that path>` after a `configFile` override), or exec
        it from your greetd/startwm entry -- on NixOS, set the NixOS
        module's `programs.scoot.session.command` to the full
        `<package>/bin/scoot --tty -- /home/<user>/.config/<that path>`
        line (an absolute path -- `Exec=` lines get no shell expansion)
        so the login-screen entry runs it instead of the `scoot-session`
        launcher. That entry runs without the launcher's session wiring
        (no `scoot-session.target`, no `graphical-session.target`, no
        activation import: the profile's user units never start there);
        for wired startup programs, use `[autostart]` instead. Null
        writes no file.
      '';
    };

    # scootbg, the wallpaper daemon a `[wallpaper]` section runs. Spelled
    # as in the NixOS module (`programs.scoot.wallpaper.*`), so either side
    # reads like the other.
    wallpaper = {
      # Follows the settings and the catalog: a `[wallpaper]` table is
      # what asks for the binary, and a catalog look with an image
      # needs it for the same reason — the look layer carries the
      # image, and the compositor runs it through scootbg. The catalog
      # half only counts with the profile (without it no catalog
      # installs). Nothing else here needs it.
      enable = lib.mkOption {
        type = lib.types.bool;
        default = hasWallpaper || ((cfg.desktop.enable or false) && lookCatalog.hasWallpaper);
        defaultText = lib.literalExpression ''
          config.programs.scoot.settings ? wallpaper || a catalog look ships an image wallpaper
        '';
        description = ''
          Install `wallpaper.package` and point `settings.wallpaper.command`
          at it (unless you set `command` yourself). On whenever `settings`
          has a `wallpaper` table or a catalog look ships an image. Set it
          to `false` to leave both alone: `[wallpaper]` then runs `scootbg`
          from PATH (a system package, say: the NixOS module's
          `programs.scoot.wallpaper`).
        '';
      };

      # `pkgs.scootbg` with the flake's overlay, else null. The flake
      # wrapper injects the flake's own scootbg on Linux (the same revision
      # as `package`) and null on Darwin, where scootbg does not build: a
      # macOS config that edits a `[wallpaper]` for a Linux box renders it
      # as written, with no `command`, and installs nothing.
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = pkgs.scootbg or null;
        defaultText = lib.literalExpression "pkgs.scootbg or null";
        example = lib.literalExpression "inputs.scoot.packages.\${pkgs.stdenv.hostPlatform.system}.scootbg";
        description = ''
          The scootbg package to install when `wallpaper.enable` is set.
          Null installs nothing and leaves `command` alone.
        '';
      };
    };

    portals = {
      # On by default behind `enable`: `scoot-portals.conf` is inert
      # until a session names `XDG_CURRENT_DESKTOP=scoot` (which the
      # compositor exports to everything it spawns), and the per-user
      # path below is the highest-precedence lookup slot, so installing
      # it changes nothing outside a scoot session. Turn it off if you
      # manage portal backends some other way.
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Install `scoot-portals.conf` to the per-user
          xdg-desktop-portal lookup path, so ScreenCast/Screenshot
          resolve to the `wlr` backend inside a scoot session.
        '';
      };
    };
  };

  config = lib.mkMerge [
    (lib.mkIf cfg.enable {
      # No assertion that `package` is set: a files-only setup (binary
      # from elsewhere, e.g. a system package) is legitimate, and with
      # defaults this still manages a (minimal, valid) config plus the
      # portals file -- both harmless. `package`'s description states
      # that null installs no binary.

      home.packages =
        lib.optional (cfg.package != null) cfg.package
        ++ lib.optional (cfg.wallpaper.enable && cfg.wallpaper.package != null) cfg.wallpaper.package;

      xdg.configFile.${cfg.configFile}.source = configFile;

      xdg.configFile.${scriptPath} = lib.mkIf (cfg.sessionScript != null) {
        executable = true;
        text = "#!/bin/sh\n" + cfg.sessionScript;
      };

      # Per-user slot from portals.conf(5), highest precedence; see
      # resources/scoot-portals.conf for what each backend line means and
      # why. This closes the remainder handed off by the session-environment
      # ticket, which this module now owns.
      xdg.configFile."xdg-desktop-portal/scoot-portals.conf" = lib.mkIf cfg.portals.enable {
        source = ../../resources/scoot-portals.conf;
      };
    })

    # `scoot-session.target` for the user units the profile owns: scoot's
    # own session scope, which every one of them starts in -- and which
    # no other desktop starts, unlike the shared
    # `graphical-session.target` it pulls in (`BindsTo`). The NixOS side
    # installs the same file beside its login entry (its
    # `systemd.user.units` carry it whenever `session.enable` is); this
    # side installs its own copy so a home-manager-only setup -- no
    # launcher, no login entry -- still has the target to start: import
    # `WAYLAND_DISPLAY`, `XDG_CURRENT_DESKTOP` and `XDG_SESSION_TYPE` into
    # the user manager, then `systemctl --user start scoot-session.target`
    # (stopped the same way on the way out; see
    # site/src/content/docs/desktop/index.md). It is the same file, not
    # a second definition: `xdg.configFile` sources
    # `resources/systemd/user/scoot-session.target` itself -- the bytes
    # the NixOS side installs verbatim -- so the two installs cannot
    # drift (a new directive in the file reaches both). The typed
    # `systemd.user.targets` option would render into that same
    # `systemd/user/scoot-session.target` path with its own INI
    # rendering (lists as duplicate keys, comments dropped), so a
    # hand-written copy there could silently diverge; the raw file is
    # byte-identical by construction. A user file wins over the system
    # one, so both paths stay identical where both are installed.
    # Present whenever a unit below can want it (any slot on, with the
    # profile or standalone); an idle file with nothing wanting it
    # starts nothing.
    (lib.mkIf
      (
        cfg.desktop.enable
        || cfg.desktop.idle.enable
        || cfg.desktop.notifications.enable
        || cfg.desktop.clipboard.enable
        || cfg.desktop.audio.enable
        || cfg.desktop.nightlight.enable
        || cfg.desktop.displays.enable
        || cfg.desktop.automount.enable
        || cfg.desktop.inputMethod.enable
      )
      {
        xdg.configFile."systemd/user/scoot-session.target".source =
          ../../resources/systemd/user/scoot-session.target;
      }
    )

    # The desktop profile's user half. `enable` owns the session-adjacent
    # defaults this side has (the portal config; the session entry and the
    # system packages are the NixOS side's, the bar is the bar module's,
    # which reads this profile). `look` themes the rendered config, each
    # leaf at `mkOptionDefault` so a value the user wrote wins and Stylix
    # (a `mkDefault` one level up) wins where present -- Stylix stays the
    # override path.
    (lib.mkIf cfg.desktop.enable {
      assertions = [
        {
          # A profile for a compositor that is not installed is
          # nonsense; without `enable` there is no session for the
          # profile's daemons to join.
          assertion = cfg.enable;
          message = ''
            programs.scoot.desktop.enable needs programs.scoot.enable:
            the profile themes and serves a scoot session, so scoot itself
            must be installed.
          '';
        }
      ];

      programs.scoot.portals.enable = lib.mkDefault true;
    })

    # The `[xwayland]` knob, in its own element: a nested `mkIf` on the leaf
    # would leave an empty `settings.xwayland` behind when off (the
    # condition empties the value, not the path), which a whole-element
    # gate does not.
    (lib.mkIf (cfg.desktop.enable && cfg.desktop.xwayland.enable) {
      programs.scoot.settings.xwayland.enabled = lib.mkDefault true;
    })

    # The default look, with the profile only: ginger-night when enabled
    # without a choice (a value the user sets wins over this default,
    # including an explicit null to theme nothing). One rung above the
    # Stylix source's `mkDefault` (1000) and below the option's own
    # `default = null` (1500): Stylix, when synthesized, beats the
    # profile, and the profile beats nothing chosen. It lives here
    # and on the NixOS side rather than as the option default, so a look
    # never lands without the profile (the refusal below keeps firing
    # then, except for the synthesized `stylix` look, which installs
    # its own catalog).
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.look = lib.mkOverride 1200 "ginger-night";
    })

    # The Stylix source, when synthesized: the declared default
    # `stylix` at `mkDefault` (1000), above the profile's `mkOverride
    # 1200` and below the user. Swapping away and back works like any
    # other look; the apps never read Stylix, only this look.
    (lib.mkIf stylixActive {
      programs.scoot.look = lib.mkDefault "stylix";
    })

    # A look without the profile is a silent no-op; refuse it loudly
    # instead (kept outside `desktop.enable` so it still fires then, the
    # way the greeter's assertions sit outside `enable`) — except the
    # synthesized `stylix` look, whose catalog installs without the
    # profile too. The catalog below installs with the profile (or that
    # synthesis) for the same reason.
    (lib.mkIf (cfg.look != null) {
      assertions = [
        {
          assertion = cfg.desktop.enable || (stylixActive && cfg.look == "stylix");
          message = ''
            programs.scoot.look needs programs.scoot.desktop.enable:
            the look is applied by the profile, so the profile must be on.
          '';
        }
        {
          # Unknown here, not at the next activation: a typo fails the
          # evaluation naming the catalog, never a half-applied switch.
          assertion = builtins.elem cfg.look lookCatalog.names;
          message = ''
            programs.scoot.look is `${cfg.look}`, which is not in programs.scoot.looks (${lib.concatStringsSep ", " lookCatalog.names}).
          '';
        }
      ];
    })

    # The bar half lives in the bar module (`nix/modules/scootbar.nix`
    # reads this profile): a set of `programs.scootbar` here would need
    # that module imported, and a conditional set of an undeclared option
    # fails eval whatever the condition is, so the profile never sets
    # across the module boundary.

    # The look catalog: each declared source linked as
    # `~/.local/share/scoot/looks/<name>`, each build-time render as
    # `~/.local/share/scoot/rendered/<name>`, plus the synthesized
    # `stylix` look when Stylix is in use. The link targets the
    # catalog's rendered entry, never a raw store path, so the next
    # generation re-resolves it. Only with the profile (or the Stylix
    # synthesis, which carries its own look) and a render binary:
    # without either, no declaration can win, so nothing is installed
    # and `scoot look sync` stays unmanaged.
    (lib.mkIf ((cfg.desktop.enable || stylixActive) && lookCatalog.sources != null) {
      home.packages = lookCatalog.fontPackages;

      xdg.dataFile = lib.listToAttrs (
        map (entry: {
          name = "scoot/looks/${entry.name}";
          value = {
            source = entry.src;
          };
        }) lookCatalog.entries
        ++ map (name: {
          name = "scoot/rendered/${name}";
          value = {
            source = lookCatalog.rendered.${name};
          };
        }) lookCatalog.names
      );

      # The declared default: the name, or an empty file for null (no
      # look layer). `sync` reads this after `linkGeneration`, so it
      # always sees the new generation's declaration.
      xdg.configFile."scoot/look-default".text = if cfg.look == null then "" else cfg.look;

      # The `org/gnome/desktop/interface` key names the merged
      # `dconf.settings` still carry — set by the user or another
      # module, since scoot defines none — so `sync` leaves those to
      # home-manager however the live value reads.
      xdg.configFile."scoot/look-nix-claims".text =
        let
          claimed = builtins.attrNames (config.dconf.settings."org/gnome/desktop/interface" or { });
        in
        lib.concatStringsSep "\n" (map (leaf: "/org/gnome/desktop/interface/${leaf}") claimed);

      # The theme targets opted out, one name per line, so the managed
      # `sync` honors them the way the eval-time theme already does: a
      # `cursor` line renders no cursor keys into the scoot fragment
      # and writes no cursor dconf keys or settings.ini cursor lines;
      # a `fonts` line resolves no font, renders no bar `font` key and
      # writes no `font-name` key, settings.ini font line or
      # fontconfig file. Empty with every target on. `sync` reads this
      # beside the declared default (see `scoot look sync`); a
      # hand-written file works the same with no Nix involved.
      xdg.configFile."scoot/look-nix-targets".text = lib.concatStringsSep "\n" (
        lib.optional (!cfg.desktop.theme.targets.cursor.enable) "cursor"
        ++ lib.optional (!cfg.desktop.theme.targets.fonts.enable) "fonts"
      );

      # The Stylix targets owning a path scoot also themes, one app per
      # line (D14): `scoot look apply` names these as "owned by Stylix's
      # target" instead of pretending the look reached them. Read with
      # the same `or` guards as the synthesis above, so an older Stylix
      # without a target, or no Stylix at all, evaluates to no lines.
      xdg.configFile."scoot/look-stylix-targets".text =
        let
          targets = ((config.stylix or { }).targets or { });
          owned = name: (((targets.${name} or { }).enable or false));
        in
        lib.concatStringsSep "\n" (
          lib.optional (owned "gtk") "gtk"
          ++ lib.optional (owned "qt") "qt"
          ++ lib.optional (owned "foot") "foot"
          ++ lib.optional (owned "mako") "mako"
          ++ lib.optional (owned "fuzzel") "fuzzel"
          ++ lib.optional (owned "swaylock") "swaylock"
          ++ lib.optional (owned "wob") "wob"
          ++ lib.optional (owned "fcitx5") "fcitx5"
          ++ lib.optional (owned "starship") "starship"
          ++ lib.optional (owned "helix") "helix"
          ++ lib.optional (owned "btop") "btop"
        );

      # The rule of section 6.3, after the links and the dconf database
      # land: home-manager's own `dconfSettings` step writes first, so
      # the toolkit step never fights it over a claimed key. Without a
      # binary on hand (a files-only setup whose `scoot` comes from
      # elsewhere), the links above still install and the sync runs
      # whenever a `scoot` is on PATH instead of failing the switch.
      home.activation.scootLookSync = {
        before = [ ];
        after = [
          "linkGeneration"
          "dconfSettings"
        ];
        data =
          if cfg.package != null then
            "${cfg.package}/bin/scoot look sync"
          else
            ''
              if command -v scoot >/dev/null 2>&1; then
                scoot look sync
              else
                echo "scoot-look-sync: no scoot binary on PATH: catalog installed, live choice untouched"
              fi
            '';
      };
    })

    # No app module reads Stylix: the compositor's `[appearance]`,
    # cursor and `[wallpaper]` theming moved into the synthesized
    # `stylix` look above (the catalog and the declared default), which
    # the runtime layer carries. A value set in `settings` still wins
    # per key over any look.
  ];
}
