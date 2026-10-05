# The `programs.scoot.desktop` option subtree, shared by the NixOS module
# (`nixos.nix`) and the home-manager module (`home.nix`): one switch plus a
# look choice for essentially a full lightweight desktop, with a
# declared-but-off slot per future piece. Each side wires only what it owns
# (NixOS: system services, backend packages, the greeter; home-manager: user
# units, binds, theme files); either side alone degrades to what it can do.
# Later children fill the slot bodies without renaming options
# (the native-replacement contract: a scoot-native piece later replaces a
# slot without changing the user's config).
#
# Takes only `lib`: no `pkgs`, so the pure modules stay usable without the
# flake's overlay, and the look wallpapers resolve to store paths wherever
# the importing module is evaluated from.
{ lib }:

let
  # One boolean plus package override per slot that installs something.
  # All default off/null: declared now so later children only fill bodies,
  # never rename options. Enabling one today is accepted and inert (no
  # packages installed, no units or files written); the description names
  # the child ticket and the default tool it will wire.
  slot =
    {
      child,
      tool,
      extra ? "",
    }:
    {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Reserved for the `${child}` child (${tool}). Off: enabling it
          today is accepted and inert -- nothing is installed and nothing
          runs yet.${extra}
        '';
      };
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        description = ''
          The package the `${child}` child installs for this slot (${tool}).
          Null installs nothing.
        '';
      };
    };

  # A slot that installs nothing, only config when its child lands
  # (a keymap, output wiring, an input-method setup): a boolean alone.
  configSlot =
    { child, does }:
    {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Reserved for the `${child}` child (${does}). Off: enabling it
          today is accepted and inert.
        '';
      };
    };
in
{
  # The option subtree, assigned to `options.programs.scoot.desktop` by
  # both modules.
  options = {
    enable = lib.mkEnableOption "the scoot desktop profile: the session wiring plus bar and wallpaper defaults, themed by `look`";

    # Null (the default) themes nothing: every value below is a default
    # the user or Stylix beats per key. An unknown value is an eval error
    # naming these three (the `enum` type's own message).
    look = lib.mkOption {
      type = lib.types.nullOr (
        lib.types.enum [
          "vinyl-sunset"
          "music-desk"
          "radial-burst"
        ]
      );
      default = null;
      example = "vinyl-sunset";
      description = ''
        One of `docs/examples/*`: applies that example's palette to every
        piece the flake owns today (the compositor `[appearance]` colors,
        the bar `colors`, the session wallpaper where one ships in the
        repository). Null themes nothing. Each value is a default a value
        you set in `settings` beats per key, and Stylix beats where
        present (Stylix stays the override path); see docs/nix.md.
      '';
    };

    # The bar half of `enable`: on, the profile enables and themes the bar
    # (through `programs.scootbar`, when that module is imported -- the
    # profile never requires it); off, the bar is entirely yours (enabled
    # or not, themed or not, the look leaves it alone). Read only with
    # `enable`: without it this does nothing.
    bar.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Whether `desktop.enable` enables and themes the bar
        (`programs.scootbar`, when imported). Set to `false` to own the
        bar yourself: the profile then neither enables it nor applies
        `look` colors to it. Read only with `desktop.enable`.
      '';
    };

    # The compositor half of XWayland support: on, the `[xwayland] enabled`
    # knob defaults on (read with `desktop.enable`). The package half stays
    # `programs.scoot.package`: point it at the flake's `scoot-xwayland`
    # (or `scoot-gpu-xwayland`) build for X11 apps -- see docs/nix.md.
    # The NixOS side accepts this and reserves it; the knob itself is
    # config-file (home-manager) wiring.
    xwayland.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Default `[xwayland] enabled` on (read with `desktop.enable`).
        Pair with an XWayland build as `programs.scoot.package` (the
        flake's `scoot-xwayland`); with the default package the knob
        warns and the session runs Wayland-only. See docs/nix.md.
      '';
    };

    # Idle policy (dim, screens off, lock before sleep, media inhibit),
    # plus the locker behind it (over `ext-session-lock-v1`).
    idle =
      (slot {
        child = "desktop-idle-lock";
        tool = "swayidle, the policy the docs already standardize on";
      })
      // {
        lock = slot {
          child = "desktop-idle-lock";
          tool = "one of swaylock, waylock, gtklock or hyprlock (all work per docs/protocols.md)";
        };
      };
    # mako now, scootnotify later without changing option names.
    notifications = slot {
      child = "desktop-notifications";
      tool = "`mako` (the daemon name is the only visible change when scootnotify replaces it)";
    };
    # fuzzel (layer-shell `overlay` native, no toolkit). Note: the
    # default binds name `wofi` today; the launcher child reconciles that.
    launcher = slot {
      child = "desktop-launcher";
      tool = "`fuzzel`";
    };
    # Screenshots bound to keys.
    capture = slot {
      child = "desktop-capture";
      tool = "`grim` plus `slurp` (`grim` 1.5.0 or later, which speaks ext-image-copy-capture)";
    };
    # The privilege prompt.
    auth = slot {
      child = "desktop-auth-secrets";
      tool = "a polkit agent";
    };
    # The keyring.
    secrets = slot {
      child = "desktop-auth-secrets";
      tool = "a secrets service";
    };
    # Pipewire/wireplumber baseline, media keys, brightness keys and an OSD.
    audio = slot {
      child = "desktop-audio-osd";
      tool = "pipewire plus wireplumber, with the bar's volume, brightness, media and microphone modules as the display half";
    };
    # Clipboard persistence plus history and a picker bind.
    clipboard = slot {
      child = "desktop-clipboard";
      tool = "`cliphist` plus `wl-clipboard`";
    };
    # Night light over `wlr-gamma-control-v1`.
    nightlight = slot {
      child = "desktop-nightlight";
      tool = "`wlsunset` (single purpose) or `gammastep` (undecided)";
    };
    # Power profiles, lid and low-battery suspend, charge limit.
    power = slot {
      child = "desktop-power";
      tool = "`power-profiles-daemon` plus logind wiring";
    };
    # GTK/Qt settings, dark-mode signal and a non-Stylix fallback.
    # Stylix stays the override path where present (as for `look`).
    theme = slot {
      child = "desktop-theme-look";
      tool = "the non-Stylix GTK/Qt theme derivation";
    };
    # The terminal the default binds spawn, and a file manager.
    # The manager is explicitly optional: nothing references one anywhere
    # today.
    apps = {
      terminal = slot {
        child = "desktop-apps";
        tool = "`foot` (already the default bind)";
      };
      fileManager = slot {
        child = "desktop-apps";
        tool = "a file manager (optional; nothing references one today)";
      };
    };
    # One keymap every other child registers into, each bind a `mkDefault`
    # so a user value wins.
    keys = configSlot {
      child = "desktop-keys";
      does = "the shared keymap: hardware keys and desktop actions";
    };
    # Output policy (scale, placement) from the connected set.
    displays = configSlot {
      child = "desktop-displays";
      does = "output policy";
    };
    # Input-method wiring, off by default like everything here.
    inputMethod = configSlot {
      child = "desktop-input-method";
      does = "input-method wiring";
    };
    # Removable-media automount. In the paved-path inventory but with no
    # child filed yet; reserved here so the shell already covers it.
    automount = slot {
      child = "a future automount child (unfiled)";
      tool = "`udiskie`";
    };
  };

  # The palettes behind `look`, read from `docs/examples/*` (the
  # `scoot.toml` `[appearance]` and `bar.toml` `[colors]` each look ships).
  # `wallpaper` is null where the look ships no image in the repository
  # (`vinyl-sunset`: its illustration is license-barred from being
  # committed, so the session shows the flat `background_color` below
  # unless the user sets `settings.wallpaper` themselves -- see
  # docs/nix.md), else the in-repo file plus its mode.
  looks = {
    vinyl-sunset = {
      appearance = {
        background_color = "#271A1F";
        focus_ring_active_color = "#E59560";
        focus_ring_inactive_color = "#423F51";
      };
      barColors = {
        background = "#271A1F";
        foreground = "#F1E3C6";
        accent = "#E59560";
        hover = "#FDC58B";
        dim = "#604F50";
        urgent = "#C76B47";
      };
      wallpaper = null;
    };
    music-desk = {
      appearance = {
        background_color = "#FCFBFB";
        focus_ring_active_color = "#3D579A";
        focus_ring_inactive_color = "#D5D7DD";
      };
      barColors = {
        background = "#FCFBFB";
        foreground = "#1A2032";
        accent = "#3D579A";
        hover = "#5D7AB0";
        dim = "#C9CBD0";
        urgent = "#EE6F5E";
      };
      wallpaper = {
        image = ../../docs/assets/wallpapers/music-desk.png;
        mode = "fill";
      };
    };
    radial-burst = {
      appearance = {
        background_color = "#241721";
        focus_ring_active_color = "#31a9e5";
        focus_ring_inactive_color = "#e36e38";
      };
      barColors = {
        background = "#241721";
        foreground = "#fdef1d";
        accent = "#31a9e5";
        dim = "#99911d";
        urgent = "#bf128d";
      };
      wallpaper = {
        image = ../../docs/assets/wallpapers/radial-burst.png;
        mode = "fill";
      };
    };
  };
}
