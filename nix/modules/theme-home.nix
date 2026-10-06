# `programs.scoot.desktop.theme` for home-manager: fonts, cursor,
# GTK/Qt settings, the dark-mode signal and the look's app files, all
# derived from `desktop.look` (with and without Stylix). The option
# shapes live in `./desktop.nix` (shared with the NixOS side); the
# `package` defaults live here because only this side has `pkgs`. See
# site/src/content/docs/desktop/index.md#app-theme ("App theme") and
# site/src/content/docs/scoot/theming.md.
#
# Precedence per key, highest first: what the user wrote (a value in
# `settings`/`theme.settings`, or an upstream module they enabled),
# Stylix where present, the look, the toolkit default. Nothing here
# runs: no daemons, only files, packages and session variables, so a
# look change reaches newly started apps with no re-login (running
# apps re-read GTK's `settings.ini` on save where the toolkit does,
# the rest on restart -- see the docs page).
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  theme = cfg.desktop.theme;

  # The one helper that reads look-derived values (never hand-mapped
  # here): the look, whether a target applies, the one cursor and icon
  # theme, and the hex spellings.
  themeLook = import ./theme-look.nix { inherit lib; };
  look = themeLook.lookFor cfg.desktop;

  # The whole theme renders while the slot is on with a look behind
  # it. Without a look there is nothing to derive from, so the slot
  # is inert (GTK/Qt/cursor stay unmanaged: the user's own configs
  # rule, the way every other standalone slot runs unthemed).
  themeOn = theme.enable && look != null;
  themed = name: themeOn && themeLook.themed cfg.desktop name;

  gtkThemed = themed "gtk";
  qtThemed = themed "qt";
  cursorThemed = themed "cursor";
  fontsThemed = themed "fonts";
  terminalThemed = themed "terminal" && look.appFiles.foot != null;
  shellThemed = themed "shell" && look.appFiles.starship != null;
  editorThemed =
    themed "editor" && look.appFiles.helixConfig != null && look.appFiles.helixTheme != null;
  monitorThemed = themed "monitor" && look.appFiles.btopConf != null;

  isLinux = pkgs.stdenv.hostPlatform.isLinux;

  # Whether the upstream home-manager module owns the same path (then
  # it wins: our file stays out, so the two never merge-conflict).
  # Each `or false` keeps this evaluating with that module absent.
  upstreamOwns = name: ((config.programs.${name} or { }).enable or false);

  # The upstream GTK and Qt modules live at the top level, not under
  # `programs` -- and each owns only the paths it actually writes, so
  # each theme file stays out exactly when that path is owned (per-key
  # precedence, not per-toolkit). Verified against home-manager master:
  # - `modules/misc/gtk.nix` + `modules/misc/gtk/gtk3.nix` /
  #   `gtk4.nix`: `gtk.enable` always writes both `settings.ini` files'
  #   `.text` (gtk3/gtk4 default on), so both inis defer to it;
  # - `gtk.css` only when that side sets `extraCss` (either version)
  #   or a `gtk4.theme` package, so the look's accents stay otherwise;
  # - the dconf `color-scheme` leaf only when `gtk3.colorScheme` is set
  #   (nulls are filtered out), so the dark-mode signal stays otherwise;
  # - `modules/misc/qt/default.nix`: `qt6ct/qt6ct.conf` only when
  #   `qt.enable` holds `qt6ctSettings`, so the generated config defers
  #   exactly then (the `colors/scoot-look.conf` scheme beside it is a
  #   path upstream never writes, so it always stays).
  # Every `or` below degrades fail-safe with that module (or leaf)
  # absent: unread ownership reads as unowned, and a real collision
  # stays a loud eval error rather than a silent half-theme.
  gtkCfg = config.gtk or { };
  gtk3Cfg = gtkCfg.gtk3 or { };
  gtk4Cfg = gtkCfg.gtk4 or { };
  gtk4Theme = gtk4Cfg.theme or null;
  # Nullable leaf: `or` only follows a selection, so the null case is
  # spelled out instead of chained.
  gtk4ThemePackage = if gtk4Theme == null then null else gtk4Theme.package or null;
  gtkOwned = gtkCfg.enable or false;
  gtkCssOwned =
    gtkOwned
    && (
      ((gtk3Cfg.extraCss or "") != "") || ((gtk4Cfg.extraCss or "") != "") || (gtk4ThemePackage != null)
    );
  gtkColorSchemeOwned = gtkOwned && ((gtk3Cfg.colorScheme or null) != null);
  qtCfg = config.qt or { };
  qtConfOwned = (qtCfg.enable or false) && ((qtCfg.qt6ctSettings or null) != null);

  # The GTK theme name from the look's polarity (Adwaita ships inside
  # GTK itself: no extra theme package, nothing invented).
  gtkTheme = if look.isDark then "Adwaita-dark" else "Adwaita";
  darkBool = if look.isDark then "true" else "false";

  # The look's app colors, derived from its palette through the one
  # helper (accent plus surfaces -- never hand-mapped here).
  app = if look == null then null else themeLook.appColors look;

  # The generated GTK `settings.ini`: the theme, the icon theme, the
  # look's faces where their targets are on, and the dark-mode
  # preference. `gtk-application-prefer-dark-theme` drives plain GTK
  # (and older libadwaita, e.g. in Flatpaks); the pinned libadwaita
  # only warns on it and follows the dconf `color-scheme` below
  # instead (verified against its 1.9.3 source: portal first, then
  # GSettings, and the key is what both read). Merged as an attrset
  # (the mako pattern), so a value in `theme.settings` wins per key
  # instead of doubling the line.
  gtkIni = pkgs.writeText "scoot-look-settings.ini" (
    lib.concatStringsSep "\n" (
      [
        "# Generated by programs.scoot.desktop.theme -- see https://www.scoot.sh/scoot/theming/."
        "[Settings]"
      ]
      ++ lib.mapAttrsToList (name: value: "${name}=${value}") (
        {
          gtk-theme-name = gtkTheme;
          gtk-icon-theme-name = themeLook.iconTheme;
        }
        // lib.optionalAttrs fontsThemed { gtk-font-name = "${look.fonts.sans} 11"; }
        // lib.optionalAttrs cursorThemed {
          gtk-cursor-theme-name = themeLook.cursor.name;
          gtk-cursor-theme-size = toString themeLook.cursor.size;
        }
        // {
          gtk-application-prefer-dark-theme = darkBool;
          gtk-xft-antialias = "1";
          gtk-xft-hinting = "1";
        }
        // theme.settings
      )
    )
    + "\n"
  );

  # The generated GTK `gtk.css` (both versions, same content): the
  # look's accent and surfaces as the named colors the pinned
  # libadwaita honors (verified against its 1.9.3 source), so a
  # selected row or slider reads as the look's accent instead of
  # Adwaita's default blue. Two blocks, both required:
  #
  # - `@define-color` names: what plain GTK widgets and the compat
  #   aliases (`theme_selected_bg_color` etc.) resolve. Plain GTK3
  #   honors them in `gtk.css` the same way.
  # - a `:root` block with the matching `--var` names: what libadwaita
  #   widgets actually paint with (`button.suggested-action` reads
  #   `var(--accent-bg-color)`, and the `:root` defaults live in
  #   libadwaita's own stylesheet at THEME priority, so names alone
  #   never reach them -- proven live: names-only rendered stock blue
  #   while the dark stylesheet applied). Same-specificity `:root`
  #   ties lose to this file's USER priority, and the literal hexes
  #   (never `@`-references, which resolve per provider) make the win
  #   unconditional. Backdrop and dialog pairs alias the window pair
  #   the way the stock sheet aliases `@window_bg_color`; shade and
  #   outline colors stay stock (black-alpha overlays, independent of
  #   the background).
  #
  # Semantic colors (destructive, success, warning, error) stay the
  # toolkit defaults: the look names no hues for them, and inventing
  # any would break the derive-don't-invent rule the static app files
  # follow.
  gtkCss = pkgs.writeText "scoot-look-gtk.css" ''
    /* Generated by programs.scoot.desktop.theme -- see https://www.scoot.sh/scoot/theming/. */
    @define-color accent_bg_color ${app.accent};
    @define-color accent_fg_color ${app.accentFg};
    @define-color accent_color ${app.accent};
    @define-color window_bg_color ${app.windowBg};
    @define-color window_fg_color ${app.windowFg};
    @define-color view_bg_color ${app.viewBg};
    @define-color view_fg_color ${app.viewFg};
    @define-color headerbar_bg_color ${app.headerbarBg};
    @define-color headerbar_fg_color ${app.headerbarFg};
    @define-color headerbar_border_color ${app.dim};
    @define-color popover_bg_color ${app.windowBg};
    @define-color popover_fg_color ${app.windowFg};
    @define-color card_bg_color ${app.windowBg};
    @define-color card_fg_color ${app.windowFg};
    @define-color sidebar_bg_color ${app.windowBg};
    @define-color sidebar_fg_color ${app.windowFg};
    :root {
      --accent-bg-color: ${app.accent};
      --accent-fg-color: ${app.accentFg};
      --accent-color: ${app.accent};
      --window-bg-color: ${app.windowBg};
      --window-fg-color: ${app.windowFg};
      --view-bg-color: ${app.viewBg};
      --view-fg-color: ${app.viewFg};
      --headerbar-bg-color: ${app.headerbarBg};
      --headerbar-fg-color: ${app.headerbarFg};
      --headerbar-border-color: ${app.dim};
      --headerbar-backdrop-color: ${app.windowBg};
      --popover-bg-color: ${app.windowBg};
      --popover-fg-color: ${app.windowFg};
      --card-bg-color: ${app.windowBg};
      --card-fg-color: ${app.windowFg};
      --sidebar-bg-color: ${app.windowBg};
      --sidebar-fg-color: ${app.windowFg};
      --sidebar-backdrop-color: ${app.windowBg};
      --dialog-bg-color: ${app.windowBg};
      --dialog-fg-color: ${app.windowFg};
    }
  '';

  # A `#rrggbb` look token as opaque `#AARRGGBB` (what qt6ct color
  # scheme entries spell, lowercase like its shipped schemes).
  qtArgb = color: "ff${lib.toLower (lib.removePrefix "#" color)}";
  # The same token translucent (placeholder text, the stock-scheme
  # shape: `#80ffffff` on dark schemes, `#80000000` on light ones).
  qtArgbSoft = color: "80${lib.toLower (lib.removePrefix "#" color)}";

  # The generated Qt color scheme: the look's palette in qt6ct's own
  # `[ColorScheme]` file shape (pinned qt6ct 0.11
  # `src/qt6ct-common/qt6ct.cpp:loadColorScheme`: `active_colors`,
  # `inactive_colors`, `disabled_colors`, each 21 entries in
  # `QPalette::ColorRole` order -- verified against its shipped
  # `colors/simple.conf` (light) and `colors/darker.conf` (dark), with
  # ButtonText=8, Base=9, Window=10). Text roles carry the look's
  # foreground, surfaces its background, Highlight and Link its
  # accent with the background on top (the AA pairing the content
  # checks measure); inactive mirrors active the way the stock schemes
  # do, and disabled drops the text roles to the look's dim, which is
  # the conventional disabled signal. Dark looks get a dark palette,
  # the light one a light palette -- derived from polarity, so the
  # preview window no longer shows a light palette inside a dark
  # style. No `[Fonts]` group: absent keys fall back to the
  # application font (`readSettings` in `qt6ctplatformtheme.cpp`),
  # which follows fontconfig -- the proportional sans below -- so Qt
  # UI text is proportional with nothing hand-spelled.
  qtScheme = pkgs.writeText "scoot-look-qt6ct-colors.conf" (
    let
      fg = qtArgb app.windowFg;
      bg = qtArgb app.windowBg;
      accent = qtArgb app.accent;
      accentFg = qtArgb app.accentFg;
      dim = qtArgb app.dim;
      softFg = qtArgbSoft app.windowFg;
      softDim = qtArgbSoft app.dim;
      # Role order 0-20: WindowText Button Light Midlight Dark Mid Text
      # BrightText ButtonText Base Window Shadow Highlight
      # HighlightedText Link LinkVisited AlternateBase NoRole
      # ToolTipBase ToolTipText PlaceholderText.
      active = [
        fg
        bg
        fg
        dim
        bg
        dim
        fg
        accent
        fg
        bg
        bg
        dim
        accent
        accentFg
        accent
        accent
        bg
        fg
        bg
        fg
        softFg
      ];
      disabled = [
        dim
        bg
        fg
        dim
        bg
        dim
        dim
        dim
        dim
        bg
        bg
        dim
        dim
        dim
        accent
        accent
        bg
        fg
        bg
        dim
        softDim
      ];
      row = colors: lib.concatStringsSep ", " (map (c: "#${c}") colors);
    in
    ''
      # Generated by programs.scoot.desktop.theme -- see https://www.scoot.sh/scoot/theming/.
      [ColorScheme]
      active_colors=${row active}
      inactive_colors=${row active}
      disabled_colors=${row disabled}
    ''
  );

  # The generated qt6ct config: the Adwaita Qt style in the look's
  # polarity plus the Adwaita icon theme. Capitalized exactly as the
  # plugin registers them (`Adwaita`, `Adwaita-Dark`: verified live --
  # a lowercase `adwaita` leaves qt6ct warning that the application
  # is not configured correctly). `custom_palette` points at the
  # scheme below: the `~` expands through qt6ct's own `resolvePath`
  # (pinned 0.11 `qt6ct.cpp`: every `~` becomes the home directory),
  # the same `~/.config/qt6ct` its default scheme directory lives
  # under. Without both keys the platform theme keeps its default
  # light palette, which is what showed a light preview inside every
  # dark look.
  qtStyle = if look.isDark then "Adwaita-Dark" else "Adwaita";
  qtConf = pkgs.writeText "scoot-look-qt6ct.conf" ''
    # Generated by programs.scoot.desktop.theme -- see https://www.scoot.sh/scoot/theming/.
    [Appearance]
    style=${qtStyle}
    icon_theme=${themeLook.iconTheme}
    standard_dialogs=default
    custom_palette=true
    color_scheme_path=~/.config/qt6ct/colors/scoot-look.conf
  '';

  # The Qt plugin dirs the theme needs on `QT_PLUGIN_PATH`: the
  # platformtheme (qt6ct) and the style (Adwaita-Qt) live there, and Qt
  # only searches its compiled-in paths otherwise, so an unpackaged
  # style would silently fall back to fusion. Qt6 only: the directory
  # spelling is Qt6's. Lazy like the configs above: forcing it with a
  # null package would throw, so every use below is behind
  # `qtPluginPathReady`.
  qtPluginDirs = [
    "${theme.qt.package}/lib/qt-6/plugins"
    "${theme.qt.stylePackage}/lib/qt-6/plugins"
  ];
  qtPluginPathReady = qtThemed && theme.qt.package != null && theme.qt.stylePackage != null;

  # The fontconfig default from the look: the proportional sans
  # for sans-serif (GTK/Qt apps, the greeter, the bar's fallback) and
  # the terminal face for monospace (foot and every other terminal).
  # Written as `match` rules with strong bindings, not `<alias>`
  # blocks: the alias form parsed cleanly but registered no rules in
  # the pinned fontconfig (proven live with `fc-match`: the generic
  # still resolved to the system default with the alias file present,
  # and to the aliased face with this form). The bar's own face stays
  # the look's `ui` (its Nerd glyphs carry the module icons);
  # everything reading fontconfig sans gets the proportional face
  # instead of the code-like mono.
  fontsConf = pkgs.writeText "scoot-look-fonts.conf" ''
    <?xml version="1.0"?>
    <!DOCTYPE fontconfig SYSTEM "fonts.dtd">
    <fontconfig>
      <!-- Generated by programs.scoot.desktop.theme (see https://www.scoot.sh/scoot/theming/). -->
      <match target="pattern">
        <test name="family"><string>sans-serif</string></test>
        <edit name="family" mode="prepend" binding="strong"><string>${look.fonts.sans}</string></edit>
      </match>
      <match target="pattern">
        <test name="family"><string>monospace</string></test>
        <edit name="family" mode="prepend" binding="strong"><string>${look.fonts.mono}</string></edit>
      </match>
    </fontconfig>
  '';

  # A null beside `enable` is the loud assertion below, not a throw
  # inside `getExe`: the same guard the idle policy uses, since
  # standalone evals collect assertions without enforcing them.
  toolsReady =
    (!cursorThemed || theme.cursor.package != null)
    && (!gtkThemed || !qtThemed || theme.icon.package != null)
    && (
      !fontsThemed
      || (
        theme.fonts.uiPackage != null && theme.fonts.sansPackage != null && theme.fonts.monoPackage != null
      )
    )
    && (!qtThemed || (theme.qt.package != null && theme.qt.stylePackage != null));
in
{
  options.programs.scoot.desktop.theme = {
    # The tools below are Linux-only: their attributes refuse
    # evaluation when forced on Darwin, so `or null` alone does not
    # save them (the Darwin `nix flake check` run reads every
    # default). Off Linux each defaults to null, which the assertions
    # below refuse loudly instead of installing nothing silently.
    cursor.package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (if pkgs ? vanilla-dmz then pkgs.vanilla-dmz else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.vanilla-dmz else null";
      description = ''
        The cursor theme to install for the look (Vanilla-DMZ: the
        classic X cursor, 3.3 MiB unpacked against Bibata's 322 MiB).
        Null installs nothing. Linux-only: null off Linux.
      '';
    };

    icon.package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default =
        if isLinux then (if pkgs ? adwaita-icon-theme then pkgs.adwaita-icon-theme else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.adwaita-icon-theme else null";
      description = ''
        The icon theme to install for GTK and Qt apps (Adwaita, the
        toolkit default both expect). Null installs nothing.
        Linux-only: null off Linux.
      '';
    };

    fonts.uiPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (pkgs.nerd-fonts.droid-sans-mono or null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.nerd-fonts.droid-sans-mono else null";
      description = ''
        The look's bar face (whose Nerd glyphs carry the bar's module
        icons) as a package. Null installs nothing. Linux-only: null
        off Linux.
      '';
    };

    fonts.sansPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (pkgs.dejavu_fonts.minimal or null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.dejavu_fonts.minimal else null";
      description = ''
        The look's proportional UI sans (GTK/Qt apps, the greeter,
        fontconfig sans-serif) as a package: DejaVu Sans, already
        shipped as the bar's own fallback file, so it adds zero
        closure. Null installs nothing. Linux-only: null off Linux.
      '';
    };

    fonts.monoPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default =
        if isLinux then
          (
            if cfg.desktop.look == "radial-burst" then
              (if pkgs ? dejavu_fonts then pkgs.dejavu_fonts else null)
            else
              (pkgs.nerd-fonts.fira-code or null)
          )
        else
          null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then per-look monospace package else null";
      description = ''
        The look's terminal face (foot, fontconfig monospace) as a
        package: FiraCode Nerd Font, or DejaVu for the radial-burst
        look (whose foot config names it). Null installs nothing.
        Linux-only: null off Linux.
      '';
    };

    qt.package = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (pkgs.qt6Packages.qt6ct or null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.qt6Packages.qt6ct else null";
      description = ''
        The Qt config tool whose `qt6ct.conf` the theme writes (no
        daemon: Qt reads the file at startup). Null installs nothing.
        Linux-only: null off Linux.
      '';
    };

    qt.stylePackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then (if pkgs ? adwaita-qt6 then pkgs.adwaita-qt6 else null) else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.adwaita-qt6 else null";
      description = ''
        The Adwaita Qt style the generated `qt6ct.conf` names.
        Null installs nothing. Linux-only: null off Linux.
      '';
    };
  };

  config = lib.mkMerge [
    # Packages, assertions and session variables for the user. Each
    # piece reads its own target, so opting one out drops its files
    # (and its package) while the rest follows the look.
    (lib.mkIf themeOn {
      assertions = [
        {
          assertion = !cursorThemed || theme.cursor.package != null;
          message = ''
            programs.scoot.desktop.theme.targets.cursor.enable is set but
            programs.scoot.desktop.theme.cursor.package is null: set it
            explicitly (apply the overlay, or point at a cursor theme).
          '';
        }
        {
          assertion = !(gtkThemed || qtThemed) || theme.icon.package != null;
          message = ''
            programs.scoot.desktop.theme.targets.gtk/qt.enable is set but
            programs.scoot.desktop.theme.icon.package is null: set it
            explicitly (apply the overlay, or point at an icon theme).
          '';
        }
        {
          assertion = !fontsThemed || theme.fonts.uiPackage != null;
          message = ''
            programs.scoot.desktop.theme.targets.fonts.enable is set but
            programs.scoot.desktop.theme.fonts.uiPackage is null: set it
            explicitly (apply the overlay, or point at the look's UI font).
          '';
        }
        {
          assertion = !fontsThemed || theme.fonts.sansPackage != null;
          message = ''
            programs.scoot.desktop.theme.targets.fonts.enable is set but
            programs.scoot.desktop.theme.fonts.sansPackage is null: set it
            explicitly (apply the overlay, or point at the look's UI sans).
          '';
        }
        {
          assertion = !fontsThemed || theme.fonts.monoPackage != null;
          message = ''
            programs.scoot.desktop.theme.targets.fonts.enable is set but
            programs.scoot.desktop.theme.fonts.monoPackage is null: set it
            explicitly (apply the overlay, or point at the look's terminal font).
          '';
        }
        {
          assertion = !qtThemed || theme.qt.package != null;
          message = ''
            programs.scoot.desktop.theme.targets.qt.enable is set but
            programs.scoot.desktop.theme.qt.package is null: set it
            explicitly (apply the overlay, or point at a qt6ct).
          '';
        }
        {
          assertion = !qtThemed || theme.qt.stylePackage != null;
          message = ''
            programs.scoot.desktop.theme.targets.qt.enable is set but
            programs.scoot.desktop.theme.qt.stylePackage is null: set it
            explicitly (apply the overlay, or point at an Adwaita Qt style).
          '';
        }
      ];

      home.packages =
        lib.optional (cursorThemed && theme.cursor.package != null) theme.cursor.package
        ++ lib.optional ((gtkThemed || qtThemed) && theme.icon.package != null) theme.icon.package
        ++ lib.optional (fontsThemed && theme.fonts.uiPackage != null) theme.fonts.uiPackage
        ++ lib.optional (fontsThemed && theme.fonts.sansPackage != null) theme.fonts.sansPackage
        ++ lib.optional (fontsThemed && theme.fonts.monoPackage != null) theme.fonts.monoPackage
        ++ lib.optional (qtThemed && theme.qt.package != null) theme.qt.package
        ++ lib.optional (qtThemed && theme.qt.stylePackage != null) theme.qt.stylePackage;

      # Session variables for the systemd user session (user units and
      # D-Bus-activated apps inherit the manager environment): the Qt
      # platform theme that reads the config below plus the plugin path
      # its platformtheme and style live on (Qt only searches its
      # compiled-in paths otherwise, so an unpackaged style would
      # silently fall back), and the X cursor for X11/XWayland apps
      # (the Wayland cursor comes from the compositor config beside
      # this). Each at `mkDefault`, so an explicit value still wins.
      # `QT_PLUGIN_PATH` composes one layer down as well (see the
      # `sessionSearchVariables` element below): the manager file
      # (`environment.d`) joins one string per variable, so a user
      # plain value wins there -- the same replacement home-manager's
      # own qt module uses (`modules/misc/qt/default.nix` joins its
      # profile dirs the same way) -- while shells prepend the theme
      # dirs beside the user's own value instead of dropping them.
      # Qt6 only: the plugin directory spelling is Qt6's.
      #
      # Deliberately no `GTK_THEME`: it names a theme GTK must find,
      # and on GTK4/libadwaita its presence makes libadwaita skip
      # installing its own stylesheet providers entirely (pinned 1.9.3
      # `adw-style-manager.c`: without `GTK_THEME` it forces
      # `Adwaita-empty` plus its theme, accent and fonts providers),
      # leaving plain-GTK fallback rendering the `gtk.css` cannot
      # reach. `settings.ini` already names the theme (`Adwaita` /
      # `Adwaita-dark`) everywhere a name is read, so nothing is lost.
      systemd.user.sessionVariables = lib.mkMerge [
        (lib.mkIf qtThemed { QT_QPA_PLATFORMTHEME = lib.mkDefault "qt6ct"; })
        (lib.mkIf qtPluginPathReady {
          QT_PLUGIN_PATH = lib.mkDefault (lib.concatStringsSep ":" qtPluginDirs);
        })
        (lib.mkIf cursorThemed {
          XCURSOR_THEME = lib.mkDefault themeLook.cursor.name;
          XCURSOR_SIZE = lib.mkDefault (toString themeLook.cursor.size);
        })
      ];

      # The same plugin dirs for shells: `home.sessionSearchVariables`
      # prepends each entry before the existing `$QT_PLUGIN_PATH` at
      # login (`prependToVar` in home-manager's `modules/lib/shell.nix`:
      # `dirs...${VAR:+:}...$VAR`), so a user's own value -- for an
      # input method, say -- keeps working beside the theme's instead
      # of dropping it. Lists concatenate across modules, so a user
      # list and the upstream qt module's own profile dirs all land.
      home.sessionSearchVariables = lib.mkIf qtPluginPathReady {
        QT_PLUGIN_PATH = qtPluginDirs;
      };
    })

    # One element per file below (whole elements, not `source`-only
    # gates: a `source = mkIf false` still registers the file's key,
    # so opting a target out would leave its path behind as an empty
    # file entry).

    # GTK reads `settings.ini` on Wayland and on X11 alike (no
    # daemon, no re-login: newly started apps pick it up; running
    # ones re-read it where the toolkit watches it).
    (lib.mkIf (themeOn && gtkThemed && toolsReady && !gtkOwned) {
      xdg.configFile."gtk-3.0/settings.ini".source = gtkIni;
      xdg.configFile."gtk-4.0/settings.ini".source = gtkIni;
    })

    # `gtk.css` beside it carries the look's named colors (libadwaita
    # and plain GTK3 alike read it at startup). It stays out only when
    # the upstream module writes its own css (see `gtkCssOwned`
    # above); a `gtk.enable` that only sets other keys keeps the
    # look's accents.
    (lib.mkIf (themeOn && gtkThemed && toolsReady && !gtkCssOwned) {
      xdg.configFile."gtk-3.0/gtk.css".source = gtkCss;
      xdg.configFile."gtk-4.0/gtk.css".source = gtkCss;
    })

    # The dark-mode signal libadwaita actually follows: GSettings
    # `org.gnome.desktop.interface color-scheme` (`prefer-dark` for a
    # dark look, `prefer-light` for the light one), written to the
    # user's dconf database at activation. No daemon: the dconf client
    # library reads the database file directly, and a settings portal
    # (where one runs) reports this same key -- so one write covers
    # the portal-less session, the portal-backed session, and anything
    # in between. Without it a dark look's libadwaita apps render
    # light (proven live: `prefer-dark-theme` alone only warns). It
    # stays out only when the upstream module writes the same leaf
    # (see `gtkColorSchemeOwned`); its other dconf leaves merge beside
    # this one.
    (lib.mkIf (themeOn && gtkThemed && toolsReady && !gtkColorSchemeOwned) {
      dconf.settings."org/gnome/desktop/interface".color-scheme =
        if look.isDark then "prefer-dark" else "prefer-light";
    })

    # Qt reads `qt6ct.conf` at startup through
    # `QT_QPA_PLATFORMTHEME=qt6ct` above (no daemon either). The
    # generated config stays out only when the upstream module writes
    # that same path (see `qtConfOwned`); the scheme beside it is a
    # path upstream never writes, so it always stays.
    (lib.mkIf (themeOn && qtThemed && toolsReady && !qtConfOwned) {
      xdg.configFile."qt6ct/qt6ct.conf".source = qtConf;
    })
    (lib.mkIf (themeOn && qtThemed && toolsReady) {
      xdg.configFile."qt6ct/colors/scoot-look.conf".source = qtScheme;
    })

    # The fontconfig default from the look.
    (lib.mkIf (themeOn && fontsThemed && toolsReady) {
      xdg.configFile."fontconfig/conf.d/10-scoot-look.conf".source = fontsConf;
    })

    # The look's own app files, from the flake instead of
    # copy-paste. Each stays the example's static file (hand-tuned
    # palettes, not generated -- see `desktop.nix`): installing
    # them is what applies them. An upstream home-manager module
    # the user enabled owns the same path and wins (our file stays
    # out, so the two never merge-conflict).
    (lib.mkIf (themeOn && terminalThemed && toolsReady && !upstreamOwns "foot") {
      xdg.configFile."foot/foot.ini".source = look.appFiles.foot;
    })
    (lib.mkIf (themeOn && shellThemed && toolsReady && !upstreamOwns "starship") {
      xdg.configFile."starship.toml".source = look.appFiles.starship;
    })
    (lib.mkIf (themeOn && editorThemed && toolsReady && !upstreamOwns "helix") {
      xdg.configFile."helix/config.toml".source = look.appFiles.helixConfig;
      # The theme beside the config, under its own name (which is
      # what the config's `theme = "..."` names).
      xdg.configFile."helix/themes/${builtins.baseNameOf look.appFiles.helixTheme}".source =
        look.appFiles.helixTheme;
    })
    (lib.mkIf (themeOn && monitorThemed && toolsReady && !upstreamOwns "btop") {
      xdg.configFile."btop/btop.conf".source = look.appFiles.btopConf;
    })
    # The btop theme beside the config, under its own name (which is
    # what the config's `color_theme` names); music-desk ships no
    # theme file (its `color_theme` stays commented out).
    (lib.mkIf
      (themeOn && monitorThemed && toolsReady && !upstreamOwns "btop" && look.appFiles.btopTheme != null)
      {
        xdg.configFile."btop/themes/${builtins.baseNameOf look.appFiles.btopTheme}".source =
          look.appFiles.btopTheme;
      }
    )

    # The profile turns the slot on (still individually disable-able
    # at plain priority, the way the clipboard slot works). Without a
    # look the slot is inert (see `themeOn` above).
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.theme.enable = lib.mkDefault true;
    })
  ];
}
