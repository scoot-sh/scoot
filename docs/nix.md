# Using scoot from Nix: flake consumption, home-manager, NixOS

- [Consuming the flake](#consuming-the-flake)
- [Platform notes](#platform-notes)
- [GPU tiers from the flake](#gpu-tiers-from-the-flake)
- [Home-manager module](#home-manager-module)
- [NixOS module](#nixos-module)
- [Migrating from a hand-rolled packaging](#migrating-from-a-hand-rolled-packaging)
- [Settings failure modes](#settings-failure-modes)
- [Reference: live defaults](#reference-live-defaults)

## Consuming the flake

In your own flake:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/<your-rev>";
    scoot.url = "github:scoot-sh/scoot";
  };
}
```

Then, in a module:

```nix
# The binary, anywhere a package list goes:
environment.systemPackages = [ inputs.scoot.packages.${pkgs.system}.scoot ];
# ... or build-and-run in one step, without installing:
# nix run github:scoot-sh/scoot -- --headless -- foot
```

`packages.<system>.scoot` is the compositor alone (`$out/bin` carries only
the `scoot` binary, with the `scoot msg` client alias kept on it),
`packages.<system>.scoot-gpu` the same binary with the `gpu-scanout` build
feature (see [GPU tiers](#gpu-tiers-from-the-flake) below),
`packages.<system>.scootctl` the standalone
remote-control client, and `packages.<system>.default` is whichever is
honest on that system (see [Platform notes](#platform-notes)). `apps`
mirrors all three (`nix run . -- ...`, `nix run .#scootctl -- ...`,
`nix run .#scoot-gpu -- --tty -- ...`).

## Platform notes

The flake covers **Apple Silicon and Linux** (`aarch64-linux`,
`x86_64-linux`, `aarch64-darwin`). On **macOS** the build gives the
`scootctl` remote-control client only — enough to drive a compositor
running in a VM; there is no macOS compositor (see README's "No macOS
adapter"). An Intel Mac is outside what the flake provides but nothing
Intel-specific blocks the client: `cargo build -p scootctl` from source
works there.

The modules below follow the same split: the home-manager module
manages the config file on any system (useful on macOS too, to keep the
config you deploy to a Linux box next to the machine that edits it),
while the NixOS module's session entry only means anything on NixOS.

## GPU tiers from the flake

Two packages, matching the two GPU tiers in
[tty.md](tty.md#which-renderer-draws-the-frames):

- `packages.<system>.scoot` (the default on Linux) composites on the CPU
  with pixman and needs no GPU at all. `--renderer gles` is available from
  this package -- the offscreen tier, still read back to main memory -- but
  it needs EGL drivers from the **host OS**, not from the derivation: the
  package ships libglvnd (the dispatch library Smithay `dlopen`s) while
  Mesa's vendor ICDs come from the system (on NixOS, `hardware.graphics`
  enabled). That split is the standard nixpkgs pattern, and it is
  deliberate: bundling Mesa would risk shadowing the host's drivers --
  notably Asahi's -- with wrong ones. On a box with no working EGL,
  `--renderer gles` is a loud startup error naming the pixman default, not
  a panic and not a silent downgrade (previously it panicked in Smithay's
  `dlopen`; gh #177).
- `packages.<system>.scoot-gpu` adds the `gpu-scanout` Cargo feature: under
  `--tty --renderer gles` the frame is composited straight into the buffer
  the CRTC scans out, with no read-back. Same binary name (`scoot`), the
  same EGL-driver requirement as above, plus a real DRM seat. Scanout
  drives the primary plane only and has never run on a real GPU -- every
  measurement so far is a software rasteriser's (see
  [Asahi.md](../Asahi.md)'s Test 4, which is what settles that).

## Home-manager module

```nix
# In your home-manager flake inputs: scoot.url = "github:scoot-sh/scoot";
# In your home configuration:
imports = [ inputs.scoot.homeModules.scoot ];
# (Legacy spelling `inputs.scoot.homeManagerModules.scoot` still resolves
# to the same module.)

programs.scoot = {
  enable = true;
  # `package` defaults to the flake's own build for your system on Linux
  # (the same per-system default as `packages`); on macOS it defaults to
  # null (files only — the module manages the config, installs no
  # binary). Set it only to override:
  # package = inputs.scoot.packages.${pkgs.system}.scoot;
  # # macOS, to also install the remote-control client:
  # package = inputs.scoot.packages.${pkgs.system}.scootctl;
  settings = {
    layout.gap = 8;
    binds = {
      "super+t" = "spawn foot";
      "ctrl+alt+space" = "spawn wofi --show drun";
    };
    autostart.commands = [ "spawn waybar" ];
  };
};
```

| Option | Default | Meaning |
|---|---|---|
| `enable` | `false` | Manage scoot files (and install `package`). |
| `package` | flake's own build (Linux), `null` (macOS) | The binary installed to your profile. `null` installs no binary (files only). |
| `settings` | `{ }` | Free-form config, rendered verbatim to TOML (see below). Empty renders a valid minimal file: the compositor runs it as pure defaults. |
| `configFile` | `"scoot/config.toml"` | Where the rendered TOML lands, relative to `$XDG_CONFIG_HOME`. Keep the default unless you pass the same path via `--config` wherever you launch scoot. |
| `sessionScript` | `null` | Startup script text, written executable beside the rendered config at `<dirOf configFile>/session.sh` (`scoot/session.sh` with the default `configFile`). Launch it with `scoot -- ~/.config/scoot/session.sh` for the default (or `~/.config/<that path>` after a `configFile` override), or exec it from your greetd/startwm entry — on NixOS, that launch line goes in the NixOS module's `session.command` (see below), which is what makes the greeter entry run this script instead of a bare compositor. `null` writes no file. |
| `portals.enable` | `true` | Install `scoot-portals.conf` to the per-user xdg-desktop-portal lookup path (`~/.config/xdg-desktop-portal/`), so ScreenCast/Screenshot resolve to the `wlr` backend inside a scoot session. Inert outside one (nothing reads it until `XDG_CURRENT_DESKTOP=scoot`). Turn off if you manage portal backends some other way. |

The example above renders byte-for-byte to:

```toml
[autostart]
commands = ["spawn waybar"]

[binds]
"ctrl+alt+space" = "spawn wofi --show drun"
"super+t" = "spawn foot"

[layout]
gap = 8
```

(That TOML is the module's own output for those settings, captured from
a real evaluation with the one invalid test bind removed — keys render
alphabetically, hence the table order.)

**Why free-form?** `settings` is a TOML value, not a typed schema per
field — deliberately. The config moves fast (three tables landed this
month); a typed schema goes stale and then lies about what the
compositor accepts, while free-form never drifts. `[binds]` is arbitrary
keys anyway, so a schema would need an escape hatch exactly where most
user content lives. Field names, types and defaults are documented in
[configuration.md](configuration.md); the full live-defaults reference
is at the bottom of this page, pasted from a real
`scoot --print-default-config` emission, not hand-written.

After `home-manager switch`, apply the new settings without re-logging in:
`scoot msg reload` (`scootctl reload` is the same client) re-reads the file
and re-applies what can move live; startup-only fields are refused with a
message rather than silently kept (see
[configuration.md](configuration.md#reloading-the-config)).

## NixOS module

```nix
# In your NixOS flake inputs: scoot.url = "github:scoot-sh/scoot";
# In your system configuration:
imports = [ inputs.scoot.nixosModules.scoot ];

programs.scoot = {
  enable = true;
  # package defaults to the flake's own scoot build, like above.
  session.enable = true;   # opt-in login-screen entry (default false)
  # Run the home-manager session script from the greeter entry
  # (pairs with `sessionScript` above; default null is a bare `--tty`):
  # session.command = "${config.programs.scoot.package}/bin/scoot --tty -- /home/alice/.config/scoot/session.sh";
};
```

| Option | Default | Meaning |
|---|---|---|
| `enable` | `false` | Install `package` system-wide. |
| `package` | flake's own `scoot` build | The binary to install and to launch from the session entry. |
| `session.enable` | `false` | Add a scoot entry to the display-manager/greetd session menu. |
| `session.command` | `null` | Full `Exec=` line for the session entry. `null` renders the bare `<package>/bin/scoot --tty` (existing configs unchanged). Set it to run something inside the session — usually `<package>/bin/scoot --tty -- <command>`, e.g. the home-manager `sessionScript` output (`/home/alice/.config/scoot/session.sh` for a user `alice` with defaults), or a wrapper script path that launches scoot itself (logging, environment setup). |

The session entry **adds a session alongside existing ones, never
replacing the default**: the module writes a `scoot.desktop`
(`Exec=<package>/bin/scoot --tty` by default, `DesktopNames=scoot` — which is what
sets `XDG_CURRENT_DESKTOP=scoot` when launched from a display manager)
into `services.displayManager.sessionPackages`, the additive list
greetd/tuigreet, GDM and SDDM read. With `session.command` set, `Exec=`
is that string verbatim — the pairing with the home-manager module is
`session.command = "<package>/bin/scoot --tty --
/home/alice/.config/scoot/session.sh"` (for a user `alice`; substitute
your own username), so the greeter starts the compositor with
your session script instead of a bare one; a wrapper script path works
the same way (the wrapper launches scoot itself). The value renders
verbatim, so desktop-entry quoting (spaces, quotes, pipes) is yours to
get right — copy the example shape. `Exec=` lines get no shell
expansion (`~` and `$HOME` arrive literally — `~` is additionally
reserved by the Desktop Entry Spec), so always spell the script out as
an absolute path, quoted per the spec. It never sets
`services.displayManager.defaultSession` (the pre-select) or
`services.greetd.settings.default_session` / `initial_session` (what
actually runs), so enabling it puts scoot in the menu next to your
existing sessions and switches nothing. `session.enable` still defaults
to **off**: a login-screen change is a change to your way back into your
own desktop, and that stays an explicit opt-in.

One auto-login caveat, verified against the pinned nixpkgs source: with
no explicit `defaultSession`, NixOS falls back to the head of the
session list for the autologin target — so on a box that auto-logs-in,
adding *any* session package can move that target. If you use
`autoLogin`, pin `services.displayManager.defaultSession` explicitly.

A complete session, as one config — the home-manager side declares the
config and the startup script, the NixOS side points the greeter entry at
that script:

```nix
# Home configuration (user `alice`):
programs.scoot = {
  enable = true;
  settings = {
    output.scale = 2.0;
    binds."super+t" = "spawn foot";
  };
  sessionScript = ''
    waybar &
    exec foot
  '';
};

# System configuration:
programs.scoot = {
  enable = true;
  session.enable = true;
  session.command = "${config.programs.scoot.package}/bin/scoot --tty -- /home/alice/.config/scoot/session.sh";
};
```

The greeter starts scoot with the session script instead of a bare
compositor. The script path is absolute (`Exec=` lines get no shell
expansion), and it is the default `<dirOf configFile>/session.sh` — a
`configFile` override moves both halves, so keep them paired.

The config file itself is per-user, so it stays in the home-manager
module above — the NixOS module owns the binary and the login entry,
nothing else. (Direct-module users, not via this flake: set
`programs.scoot.package` explicitly on both sides; there is no overlay,
so no `pkgs.scoot` exists to default to, and the NixOS module fails
loudly at eval instead of writing a session entry with no binary.)

## Migrating from a hand-rolled packaging

Converting an existing `flexwm` setup to the flake is three renames, all
silent at build time:

- **`${pkg}/bin/flexwm` → `${pkg}/bin/scoot`** in any
  `writeShellScriptBin` wrapper or `Exec=` line. Nix interpolates the store
  path without checking the binary exists, so a stale path builds fine and
  fails at the login screen.
- **`xdg.configFile."flexwm/config.toml"` → `programs.scoot.settings`**
  (or `"scoot/config.toml"`). This is the dangerous one: scoot only reads
  the `scoot` path, and a missing file at the default path is silent by
  design (see
  [configuration.md](configuration.md#failure-semantics)) — so a stale path
  boots happily on built-in defaults, silently dropping scale and binds.
  Move the content into `settings` and delete the old entry, otherwise your
  real config sits orphaned at a path nothing reads while the module renders
  defaults where scoot looks.
- **The module split**: `homeModules.scoot` (the legacy spelling
  `homeManagerModules.scoot` still resolves) owns the config file;
  `nixosModules.scoot` owns the binary and the login-screen entry. A
  hand-rolled `xdg.configFile` next to the module manages a file scoot
  never reads — keep the module's and delete the hand-rolled one.

## Settings failure modes

Two cases, behaving differently by design:

- **A value with no TOML representation** (e.g. a Nix function in
  `settings`) fails the option type-check at evaluation time ("not of type
  'TOML value'"), so the error aborts before anything builds, let alone
  starts a session. Loud and early.
- **A value that renders but has the wrong scoot type** (a string for
  `layout.gap`) builds fine and is refused at session start — where the
  loader fails safe: the whole file is discarded for built-in defaults,
  the error is logged, and the session still boots. A typo costs the
  config, never the session. (This is why `settings` needs no
  build-time type schema to be safe: see
  [configuration.md](configuration.md#failure-semantics) for the full
  refusal rules.)

## Reference: live defaults

Every key below is present in `settings` exactly as shown (uncommented;
values are the live defaults). Pasted from a real
`scoot --print-default-config` emission on 2026-09-20 (dev-VM Linux
build of `main` at `82371df`), not hand-written — regenerate rather
than edit by hand if it ever looks stale:

Two keys describe less than the packaged binary decides: setting
`backend = "gles"` needs EGL drivers from the host OS (on NixOS,
`hardware.graphics` enabled) and is a loud startup error without them,
and the `--tty` scanout tier needs `packages.scoot-gpu` instead of
`packages.scoot` — see [GPU tiers from the
flake](#gpu-tiers-from-the-flake). `[tty] gpu` is unaffected by packaging
either way: it names a host DRM device path, not a build feature.

```toml
[layout]
# Gap between columns, between windows stacked in a column, and at output edges.
gap = 12
# Column widths as fractions of the output width, in cycle order.
column_widths = [0.3333333333333333, 0.5, 0.6666666666666666]
# Index into column_widths for newly created columns.
default_column_width = 1

[appearance]
# Ring thickness; clamped to at most half of gap.
focus_ring_width = 3
focus_ring_active_color = "#6ba6fa"
focus_ring_inactive_color = "#595961"
background_color = "#14141a"
cursor_size = 16
cursor_color = "#ffffff"
# Unset follows $XCURSOR_THEME, then "default"; name one here only to override that.
cursor_theme = "Adwaita"
prefer_no_csd = true

[output]
# Output scale advertised to clients and rendered at.
scale = 1.0

[renderer]
# Which renderer composites each frame. --renderer wins over this when both name one.
backend = "pixman"

[tty]
# Unset means the automatic search picks; --gpu PATH wins over this when both name one.
# Name the display controller (prefer a stable /dev/dri/by-path/... alias):
gpu = "/dev/dri/card0"

[autostart]
# Action strings to run once each, in file order, at session startup.
commands = []

[binds]
"super+h" = "focus-column left"
"super+l" = "focus-column right"
"super+j" = "focus-window down"
"super+k" = "focus-window up"
"super+shift+h" = "move-column left"
"super+shift+l" = "move-column right"
"super+shift+j" = "move-window down"
"super+shift+k" = "move-window up"
"super+alt+h" = "consume-or-expel left"
"super+alt+l" = "consume-or-expel right"
"super+ctrl+j" = "focus-workspace down"
"super+ctrl+k" = "focus-workspace up"
"super+shift+ctrl+j" = "move-window-to-workspace down"
"super+shift+ctrl+k" = "move-window-to-workspace up"
"super+r" = "cycle-column-width"
"super+q" = "close"
"super+Return" = "spawn foot"
"super+shift+e" = "quit"
"super+1" = "focus-workspace-index 0"
"super+shift+1" = "move-window-to-workspace-index 0"
"super+2" = "focus-workspace-index 1"
"super+shift+2" = "move-window-to-workspace-index 1"
"super+3" = "focus-workspace-index 2"
"super+shift+3" = "move-window-to-workspace-index 2"
"super+4" = "focus-workspace-index 3"
"super+shift+4" = "move-window-to-workspace-index 3"
"super+5" = "focus-workspace-index 4"
"super+shift+5" = "move-window-to-workspace-index 4"
"super+6" = "focus-workspace-index 5"
"super+shift+6" = "move-window-to-workspace-index 5"
"super+7" = "focus-workspace-index 6"
"super+shift+7" = "move-window-to-workspace-index 6"
"super+8" = "focus-workspace-index 7"
"super+shift+8" = "move-window-to-workspace-index 7"
"super+9" = "focus-workspace-index 8"
"super+shift+9" = "move-window-to-workspace-index 8"
```

(Differences from the raw emission are mechanical and stated: comment
lines unwrapped where the emission's prose wraps, `#`-commented keys
uncommented with their default values filled in, and the trailing note
about `--tty` VT binds folded into
[configuration.md](configuration.md#binds). The values are untouched.)
