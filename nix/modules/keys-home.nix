# `programs.scoot.desktop.keys` for home-manager: the shared keymap's
# `[binds]` plus the tools its hardware binds run. The option shapes
# live in `./desktop.nix` (shared with the NixOS side); the tool
# `package` defaults live here because only this side has `pkgs`. See
# site/src/content/docs/desktop/index.md#hardware-keys-and-desktop-actions ("Hardware keys and desktop actions").
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.scoot;
  keys = cfg.desktop.keys;

  # The desktop profile's shared option subtree and keymap table (the
  # combos live there, so this file cannot disagree with them).
  desktop = import ./desktop.nix { inherit lib; };

  # A tool by absolute store path when its package is set, else by
  # bare name from PATH (off Linux, or a direct-module setup without
  # the overlay): a missing tool then fails quietly at runtime --
  # `State::spawn` warns and reports `false`, never wedging input --
  # so the binds are always safe to render.
  binOr = name: pkg: if pkg != null then "${lib.getExe' pkg name}" else name;

  brightnessCtl = binOr "brightnessctl" keys.brightnessPackage;
  wpctl = binOr "wpctl" keys.volumePackage;
  playerctl = binOr "playerctl" keys.mediaPackage;
  fuzzel =
    if cfg.desktop.launcher.package != null then lib.getExe cfg.desktop.launcher.package else "fuzzel";
  makoctl =
    if cfg.desktop.notifications.package != null then
      lib.getExe' cfg.desktop.notifications.package "makoctl"
    else
      "makoctl";

  # The clipboard picker's tools: absolute when the clipboard child
  # names packages, bare otherwise (same fail-quiet contract as above).
  clip = cfg.desktop.clipboard;
  cliphistBin =
    if clip.managerPackage != null then lib.getExe' clip.managerPackage "cliphist" else "cliphist";
  wlCopyBin =
    if clip.wlClipboardPackage != null then
      lib.getExe' clip.wlClipboardPackage "wl-copy"
    else
      "wl-copy";
  fuzzelPickBin =
    if clip.menuPackage != null then lib.getExe' clip.menuPackage "fuzzel" else "fuzzel";
  scootBin = if cfg.package != null then lib.getExe' cfg.package "scoot" else "scoot";

  # The lock probe the picker embeds (`@SCOOT_BIN@` replaced, the way
  # `nixos.nix` substitutes the session units).
  clipboardGuard = builtins.replaceStrings [ "@SCOOT_BIN@" ] [ scootBin ] (
    builtins.readFile ./clipboard-guard.sh
  );

  # Extra cliphist flags for the picker's `list`/`decode` (a trailing
  # space when set, nothing when the default db stands -- kept as one
  # string with the space inside, so the call sites read plainly).
  clipboardDbFlags = lib.optionalString (clip.dbPath != null) "-db-path '${clip.dbPath}' ";

  # The picker's and the launcher's theme: the look's roles as
  # fuzzel CLI colors (opaque: fuzzel takes `rrggbbaa`), the same roles
  # the bar and the locker are themed from, shared through
  # `fuzzel-theme.nix` so the two menus read as one. Nothing without a
  # look (or opted out): fuzzel's own style stands.
  fuzzelTheme = import ./fuzzel-theme.nix { inherit lib; };
  clipboardThemed =
    let
      look = if cfg.desktop.look == null then null else desktop.looks.${cfg.desktop.look};
    in
    look != null && (cfg.desktop.theme.targets.clipboard.enable or true);
  clipboardThemeFlags =
    let
      look = desktop.looks.${cfg.desktop.look};
    in
    lib.optionalString clipboardThemed (fuzzelTheme look);
  launcherThemed =
    let
      look = if cfg.desktop.look == null then null else desktop.looks.${cfg.desktop.look};
    in
    look != null && (cfg.desktop.theme.targets.launcher.enable or true);
  launcherThemeFlags =
    let
      look = desktop.looks.${cfg.desktop.look};
    in
    lib.optionalString launcherThemed (fuzzelTheme look);

  # Clipboard picker: `cliphist` history through the launcher menu
  # back into the clipboard -- lines on stdin, selection on stdout (the
  # launcher slot's dmenu contract, which the launcher reuses). A
  # script because `[binds]` has no shell (whitespace-split,
  # no quoting): a pipeline is not expressible inline. Absolute tool
  # paths when the clipboard child names packages, bare names from PATH
  # otherwise (off Linux, or without the overlay): a missing tool then
  # fails quietly at runtime -- `State::spawn` warns and reports
  # `false`, never wedging input -- so the bind is always safe.
  #
  # Three guards, each load-bearing:
  # - locked: the compositor already suppresses every bind while locked;
  #   this covers a manual run (over ssh, say), refusing with a message
  #   instead of showing history behind the lock.
  # - cancel/empty: fuzzel exits nonzero on Escape, and `--no-run-if-empty`
  #   fires on an empty history -- either must leave the clipboard alone,
  #   because `wl-copy` with empty stdin *clears* the selection.
  # - byte-exact restore: the decoded bytes travel through a temp file,
  #   never command substitution (which would strip trailing newlines and
  #   corrupt an entry copied with them, the common `echo | wl-copy` shape).
  # A pick wiped mid-flight (the lock landed between list and decode)
  # decodes to nothing and is dropped the same way.
  clipboardPick = pkgs.writeShellScriptBin "scoot-clipboard-pick" ''
    ${clipboardGuard}
    if ! clipboard_unlocked; then
      echo "scoot-clipboard-pick: the session is locked -- unlock to pick from history" >&2
      exit 1
    fi
    sel="$(${cliphistBin} ${clipboardDbFlags}list | ${fuzzelPickBin} --dmenu --prompt='clipboard: ' --no-run-if-empty --only-match${clipboardThemeFlags})" || exit 0
    case "$sel" in
      "") exit 0 ;;
    esac
    tmp="$(mktemp)" || exit 1
    trap 'rm -f "$tmp"' EXIT INT TERM
    printf '%s\n' "$sel" | ${cliphistBin} ${clipboardDbFlags}decode >"$tmp" || exit 0
    [ -s "$tmp" ] || exit 0
    ${wlCopyBin} <"$tmp"
  '';

  # The launcher: fuzzel as the keymap's drun/run binds (XDG apps,
  # most-launched first, PATH executables on the run bind -- see
  # site/src/content/docs/desktop/index.md#launcher). A script because the theme travels as CLI
  # flags (seven colors from the look, the same ones the picker above
  # carries -- one menu, one palette), and a `[binds]` action gets no
  # shell to hold them beside the binary. Absolute fuzzel path when the
  # launcher child names a package (the one fuzzel the picker already
  # themes: same derivation, no second copy), bare name from PATH
  # otherwise, with the same fail-quiet contract as the picker above.
  # Extra args pass through (`"$@"`), which is what carries the run
  # bind's `--list-executables-in-path`.
  #
  # `--layer=overlay`: fuzzel's own default is `top`, which the
  # compositor hides under a fullscreen window (the mako lesson -- see
  # `notifications-home.nix`); `overlay` stays above it. Exclusive
  # keyboard is fuzzel's default and stays: nothing else takes keys
  # while the launcher is open. No lock probe here (unlike the picker):
  # while locked no `[binds]` action fires at all, so these binds never
  # run behind the lock screen -- and a manual run only lists apps.
  launcherScript = pkgs.writeShellScriptBin "scoot-launcher" ''
    exec ${fuzzel} --layer=overlay${launcherThemeFlags} "$@"
  '';

  # Screenshot scripts: dated files into `~/Pictures` (`$HOME`
  # expands here, inside the script -- a `[binds]` action gets no
  # shell and no tilde). Absolute tool paths from the
  # `desktop-capture` child (bare names off Linux, or without the
  # overlay): a missing tool then fails quietly at runtime --
  # `State::spawn` warns and reports `false`, never wedging input --
  # so the binds are always safe to render. The region picker's dim,
  # border and selection follow the look (the same roles the chooser
  # carries), slurp's own style standing without one.
  capture = cfg.desktop.capture;
  grimBin = if capture.grimPackage != null then lib.getExe' capture.grimPackage "grim" else "grim";
  slurpBin =
    if capture.slurpPackage != null then lib.getExe' capture.slurpPackage "slurp" else "slurp";
  clipCopyBin =
    if capture.wlClipboardPackage != null then
      lib.getExe' capture.wlClipboardPackage "wl-copy"
    else
      "wl-copy";
  captureThemed =
    let
      look = if cfg.desktop.look == null then null else desktop.looks.${cfg.desktop.look};
    in
    look != null && (cfg.desktop.theme.targets.capture.enable or true);
  slurpRegionFlags =
    let
      look = desktop.looks.${cfg.desktop.look};
      # Quoted: a bare `#rrggbb` would start a shell comment inside
      # the `"$(...)"` below and eat the closing paren.
      hashA = color: "'#${lib.removePrefix "#" color}ff'";
    in
    lib.optionalString captureThemed " -c ${hashA look.appearance.focus_ring_active_color} -s ${hashA look.barColors.accent}";
  captureOutput = pkgs.writeShellScriptBin "scoot-capture-output" ''
    d="$HOME/Pictures"
    mkdir -p "$d"
    ${grimBin} "$d/scoot-$(date +%Y%m%d-%H%M%S).png"
  '';
  captureRegion = pkgs.writeShellScriptBin "scoot-capture-region" ''
    d="$HOME/Pictures"
    mkdir -p "$d"
    ${grimBin} -g "$(${slurpBin}${slurpRegionFlags})" "$d/scoot-$(date +%Y%m%d-%H%M%S).png"
  '';
  captureClipboard = pkgs.writeShellScriptBin "scoot-capture-clipboard" ''
    ${grimBin} -g "$(${slurpBin}${slurpRegionFlags})" - | ${clipCopyBin}
  '';

  # The power-profile switch: `powerprofilesctl` by absolute store
  # path when the power child names its package, bare name from PATH
  # otherwise (off Linux, or without the overlay), with the same
  # fail-loud contract as the other slot scripts (a missing daemon
  # exits nonzero with usage, never silently). `cycle` rotates in
  # PPD's canonical order (power-saver, balanced, performance),
  # skipping whatever the daemon does not list: with no platform
  # driver it offers power-saver + balanced only (its placeholder),
  # so stepping to performance would fail there -- invisibly from
  # the keybind, which is why the skip lives in `cycle` while `set`
  # stays loud for terminal use. An unrecognized current profile (a
  # future fourth) lands on balanced rather than erroring. Every path
  # prints the resulting profile, so the keybind's effect is visible
  # in a terminal too. Rendered (like the bind below) only while the
  # profiles daemon is wanted (`profiles.enable`): a driverless box
  # can drop both entirely.
  pow = cfg.desktop.power;
  ppdBin =
    if pow.profiles.package != null then lib.getExe pow.profiles.package else "powerprofilesctl";
  powerProfileScript = pkgs.writeShellScriptBin "scoot-power-profile" ''
    ctl=${lib.escapeShellArg ppdBin}
    usage() { echo "usage: scoot-power-profile [status|set <profile>|cycle]" >&2; exit 2; }
    case "''${1:-status}" in
      status) exec "$ctl" get ;;
      set)
        case "''${2:-}" in
          performance|balanced|power-saver) exec "$ctl" set "$2" ;;
          *) usage ;;
        esac
        ;;
      cycle)
        current="$("$ctl" get)"
        offered="$("$ctl" list 2>/dev/null | sed -n 's/^[* ] *\(power-saver\|balanced\|performance\):.*/\1/p' | tr '\n' ' ')"
        [ -n "$offered" ] || offered="power-saver balanced performance"
        case "$current" in
          power-saver) order="balanced performance power-saver" ;;
          balanced) order="performance power-saver balanced" ;;
          performance) order="power-saver balanced performance" ;;
          *) order="balanced power-saver performance" ;;
        esac
        next=balanced
        for candidate in $order; do
          case " $offered " in
            *" $candidate "*) next="$candidate"; break ;;
          esac
        done
        "$ctl" set "$next" && "$ctl" get
        ;;
      *) usage ;;
    esac
  '';

  # Whether a slot-gated bind's slot is on.
  slotOn = name: (cfg.desktop.${name}.enable or false);

  # Whether the volume, brightness and mic-mute binds run through the
  # audio slot's OSD scripts (set the control AND show the OSD) or
  # call the tools directly (the control alone, silent): the slot on
  # Linux, where its scripts are installed. Off Linux the binds render
  # for the Linux box they deploy to, calling bare tool names.
  audioRouted = (cfg.desktop.audio.enable or false) && isLinux;
  audioScripts = cfg.desktop.audio.scripts;

  # A hardware bind: repeats while held and fires while locked (the
  # compositor's `[binds]` table form -- see
  # site/src/content/docs/scoot/keybindings.md#the-bind-grammar).
  # Exactly the volume, brightness and media binds opt into both: a held
  # key keeps stepping at the seat keyboard's own delay and rate, and the
  # keys keep working on the lock screen. Every other bind stays a plain
  # string: fire once, never locked (launcher, clipboard, lock and capture
  # must stay refused behind the lock screen).
  hwBind = action: {
    inherit action;
    repeat = true;
    allow_when_locked = true;
  };

  # One `[binds]` entry per enabled bind: the combo from the shared
  # table, the action built here. Slot-gated binds (launcher,
  # clipboard, notifications, capture) render only while their slot
  # is enabled; the rest render with the keymap. A `false` flag
  # leaves that combo unbound; a value the user sets in
  # `settings.binds` wins per key (`mkDefault` below). The volume,
  # brightness and mic-mute binds run through the audio slot's scripts
  # while it is on (control plus OSD), bare tools otherwise. The
  # profile bind additionally needs the profiles daemon wanted
  # (`profiles.enable`): without it there is nothing to cycle.
  actions =
    let
      all = {
        brightnessUp = hwBind (
          if audioRouted then "spawn ${audioScripts.brightness}/bin/scoot-brightness up" else "spawn ${brightnessCtl} -e set +5%"
        );
        brightnessDown = hwBind (
          if audioRouted then "spawn ${audioScripts.brightness}/bin/scoot-brightness down" else "spawn ${brightnessCtl} -e set 5%-"
        );
        volumeUp = hwBind (
          if audioRouted then "spawn ${audioScripts.volume}/bin/scoot-volume sink-up" else "spawn ${wpctl} set-volume @DEFAULT_AUDIO_SINK@ 5%+"
        );
        volumeDown = hwBind (
          if audioRouted then "spawn ${audioScripts.volume}/bin/scoot-volume sink-down" else "spawn ${wpctl} set-volume @DEFAULT_AUDIO_SINK@ 5%-"
        );
        volumeMute = hwBind (
          if audioRouted then "spawn ${audioScripts.volume}/bin/scoot-volume sink-mute" else "spawn ${wpctl} set-mute @DEFAULT_AUDIO_SINK@ toggle"
        );
        micMute = hwBind (
          if audioRouted then "spawn ${audioScripts.volume}/bin/scoot-volume mic-mute" else "spawn ${wpctl} set-mute @DEFAULT_AUDIO_SOURCE@ toggle"
        );
        mediaPlay = hwBind "spawn ${playerctl} play-pause";
        mediaPause = hwBind "spawn ${playerctl} pause";
        mediaStop = hwBind "spawn ${playerctl} stop";
        mediaNext = hwBind "spawn ${playerctl} next";
        mediaPrev = hwBind "spawn ${playerctl} previous";
        lock = "spawn ${cfg.desktop.idle.lock.command}";
        launcher = "spawn ${launcherScript}/bin/scoot-launcher";
        launcherRun = "spawn ${launcherScript}/bin/scoot-launcher --list-executables-in-path";
        clipboard = "spawn ${clipboardPick}/bin/scoot-clipboard-pick";
        notifDismiss = "spawn ${makoctl} dismiss";
        notifDnd = "spawn ${makoctl} mode -t do-not-disturb";
        notifHistory = "spawn ${makoctl} restore";
        powerProfile = "spawn ${powerProfileScript}/bin/scoot-power-profile cycle";
        captureOutput = "spawn ${captureOutput}/bin/scoot-capture-output";
        captureRegion = "spawn ${captureRegion}/bin/scoot-capture-region";
        captureClipboard = "spawn ${captureClipboard}/bin/scoot-capture-clipboard";
      };
      wanted = lib.filterAttrs (
        name: _:
        (keys.binds.${name}.enable or true)
        && (desktop.keymap.${name}.slot == null || slotOn desktop.keymap.${name}.slot)
        && (name != "powerProfile" || pow.profiles.enable)
      ) all;
    in
    lib.mapAttrs' (name: action: lib.nameValuePair desktop.keymap.${name}.combo action) wanted;

  isLinux = pkgs.stdenv.hostPlatform.isLinux;

  # The tools the keymap's own binds run (brightness, volume,
  # media): installed beside the binds, each overridable through its
  # `package` beside this. Slot tools (fuzzel, mako, grim, slurp,
  # cliphist) arrive with their children; the scripts above name the
  # capture child's packages by absolute store path (bare names only
  # off Linux or without the overlay).
  ownTools =
    lib.optional (keys.brightnessPackage != null) keys.brightnessPackage
    ++ lib.optional (keys.volumePackage != null) keys.volumePackage
    ++ lib.optional (keys.mediaPackage != null) keys.mediaPackage;

  # The wrapper scripts, beside the binds that call them (Linux
  # only: bare `grim`/`slurp`/`cliphist` have no meaning on Darwin,
  # where the binds render as written for the Linux box they deploy
  # to and install nothing).
  slotScripts =
    lib.optional (slotOn "launcher") launcherScript
    ++ lib.optional (slotOn "clipboard") clipboardPick
    ++ lib.optional (slotOn "power" && pow.profiles.enable) powerProfileScript
    ++ lib.optional (slotOn "capture") captureOutput
    ++ lib.optional (slotOn "capture") captureRegion
    ++ lib.optional (slotOn "capture") captureClipboard;
in
{
  options.programs.scoot.desktop.keys = {
    # The tools below are Linux-only: their attributes exist on
    # Darwin but refuse evaluation when forced, so `or null` alone
    # does not save them (the Darwin `nix flake check` run reads
    # every default). Off Linux each defaults to null and the binds
    # above fall back to bare tool names, installing nothing.
    brightnessPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then pkgs.brightnessctl or null else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.brightnessctl or null else null";
      description = ''
        The backlight tool the brightness binds run (`-e set
        <±n>%`, so low steps stay usable). Null calls it by bare
        name (the idle policy installs it beside this when it runs).
        Linux-only: null off Linux.
      '';
    };

    volumePackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then pkgs.wireplumber or null else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.wireplumber or null else null";
      description = ''
        The audio tool the volume and mute binds run (`wpctl`
        against the default sink and source; needs PipeWire
        running, which the future `desktop-audio-osd` child wires).
        Null calls it by bare name. Linux-only: null off Linux.
      '';
    };

    mediaPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = if isLinux then pkgs.playerctl or null else null;
      defaultText = lib.literalExpression "if pkgs.stdenv.hostPlatform.isLinux then pkgs.playerctl or null else null";
      description = ''
        The MPRIS tool the media binds run (the bar's media module
        speaks the same protocol). Null calls it by bare name.
        Linux-only: null off Linux.
      '';
    };
  };

  config = lib.mkMerge [
    # The keymap: its binds into `[binds]` (each a `mkDefault` a
    # user value wins over), its tools and its slot scripts beside
    # them.
    (lib.mkIf keys.enable {
      programs.scoot.settings.binds = lib.mapAttrs (_: lib.mkDefault) actions;

      home.packages = ownTools ++ lib.optionals isLinux slotScripts;
    })

    # The profile turns the keymap on (each bind still individually
    # removable through `binds.<name>.enable`, the whole map through
    # `keys.enable = false`).
    (lib.mkIf cfg.desktop.enable {
      programs.scoot.desktop.keys.enable = lib.mkDefault true;
    })
  ];
}
