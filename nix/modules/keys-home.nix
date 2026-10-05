# `programs.scoot.desktop.keys` for home-manager: the shared keymap's
# `[binds]` plus the tools its hardware binds run. The option shapes
# live in `./desktop.nix` (shared with the NixOS side); the tool
# `package` defaults live here because only this side has `pkgs`. See
# docs/nix.md ("Hardware keys and desktop actions").
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
  binOr =
    name: pkg: if pkg != null then "${lib.getExe' pkg name}" else name;

  brightnessCtl = binOr "brightnessctl" keys.brightnessPackage;
  wpctl = binOr "wpctl" keys.volumePackage;
  playerctl = binOr "playerctl" keys.mediaPackage;
  fuzzel =
    if cfg.desktop.launcher.package != null then
      lib.getExe cfg.desktop.launcher.package
    else
      "fuzzel";
  makoctl =
    if cfg.desktop.notifications.package != null then
      lib.getExe' cfg.desktop.notifications.package "makoctl"
    else
      "makoctl";

  # Clipboard picker: `cliphist` history through the launcher menu
  # back into the clipboard. A script because `[binds]` has no shell
  # (whitespace-split, no quoting): a pipeline is not expressible
  # inline. Tool names stay bare on purpose -- `cliphist`,
  # `wl-clipboard` and the menu arrive with the `desktop-clipboard`
  # child, and until then this fails quietly like any missing tool.
  # That child replaces this script keeping the bind.
  clipboardPick = pkgs.writeShellScriptBin "scoot-clipboard-pick" ''
    cliphist list | fuzzel --dmenu | cliphist decode | wl-copy
  '';

  # Screenshot scripts: dated files into `~/Pictures` (`$HOME`
  # expands here, inside the script -- a `[binds]` action gets no
  # shell and no tilde). Bare `grim`/`slurp` for the same reason as
  # the picker above: they arrive with the `desktop-capture` child,
  # which replaces these scripts keeping the binds.
  captureOutput = pkgs.writeShellScriptBin "scoot-capture-output" ''
    d="$HOME/Pictures"
    mkdir -p "$d"
    grim "$d/scoot-$(date +%Y%m%d-%H%M%S).png"
  '';
  captureRegion = pkgs.writeShellScriptBin "scoot-capture-region" ''
    d="$HOME/Pictures"
    mkdir -p "$d"
    grim -g "$(slurp)" "$d/scoot-$(date +%Y%m%d-%H%M%S).png"
  '';

  # Whether a slot-gated bind's slot is on.
  slotOn = name: (cfg.desktop.${name}.enable or false);

  # One `[binds]` entry per enabled bind: the combo from the shared
  # table, the action built here. Slot-gated binds (launcher,
  # clipboard, notifications, capture) render only while their slot
  # is enabled; the rest render with the keymap. A `false` flag
  # leaves that combo unbound; a value the user sets in
  # `settings.binds` wins per key (`mkDefault` below).
  actions =
    let
      all = {
        brightnessUp = "spawn ${brightnessCtl} -e set +5%";
        brightnessDown = "spawn ${brightnessCtl} -e set 5%-";
        volumeUp = "spawn ${wpctl} set-volume @DEFAULT_AUDIO_SINK@ 5%+";
        volumeDown = "spawn ${wpctl} set-volume @DEFAULT_AUDIO_SINK@ 5%-";
        volumeMute = "spawn ${wpctl} set-mute @DEFAULT_AUDIO_SINK@ toggle";
        micMute = "spawn ${wpctl} set-mute @DEFAULT_AUDIO_SOURCE@ toggle";
        mediaPlay = "spawn ${playerctl} play-pause";
        mediaPause = "spawn ${playerctl} pause";
        mediaStop = "spawn ${playerctl} stop";
        mediaNext = "spawn ${playerctl} next";
        mediaPrev = "spawn ${playerctl} previous";
        lock = "spawn ${cfg.desktop.idle.lock.command}";
        launcher = "spawn ${fuzzel}";
        clipboard = "spawn ${clipboardPick}/bin/scoot-clipboard-pick";
        notifDismiss = "spawn ${makoctl} dismiss";
        notifDnd = "spawn ${makoctl} mode -t do-not-disturb";
        notifHistory = "spawn ${makoctl} restore";
        captureOutput = "spawn ${captureOutput}/bin/scoot-capture-output";
        captureRegion = "spawn ${captureRegion}/bin/scoot-capture-region";
      };
      wanted = lib.filterAttrs (
        name: _:
        (keys.binds.${name}.enable or true)
        && (desktop.keymap.${name}.slot == null || slotOn desktop.keymap.${name}.slot)
      ) all;
    in
    lib.mapAttrs' (
      name: action: lib.nameValuePair desktop.keymap.${name}.combo action
    ) wanted;

  isLinux = pkgs.stdenv.hostPlatform.isLinux;

  # The tools the keymap's own binds run (brightness, volume,
  # media): installed beside the binds, each overridable through its
  # `package` beside this. Slot tools (fuzzel, mako, grim, cliphist)
  # arrive with their children; the binds above call them by bare
  # name until then.
  ownTools =
    lib.optional (keys.brightnessPackage != null) keys.brightnessPackage
    ++ lib.optional (keys.volumePackage != null) keys.volumePackage
    ++ lib.optional (keys.mediaPackage != null) keys.mediaPackage;

  # The wrapper scripts, beside the binds that call them (Linux
  # only: bare `grim`/`slurp`/`cliphist` have no meaning on Darwin,
  # where the binds render as written for the Linux box they deploy
  # to and install nothing).
  slotScripts =
    lib.optional (slotOn "clipboard") clipboardPick
    ++ lib.optional (slotOn "capture") captureOutput
    ++ lib.optional (slotOn "capture") captureRegion;
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
