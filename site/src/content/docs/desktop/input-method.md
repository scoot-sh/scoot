---
title: Input method
description: "fcitx5 for CJK and compose input (opt-in)."
---

Type CJK or composed text, off by default: if you type Chinese,
Japanese, Korean or rely on compose, enable the engine — otherwise
leave it off (most sessions type no CJK, and an engine awake in
every session would spend closure and wakeups on nothing).

```nix title="configuration.nix and home.nix"
programs.scoot.desktop = {
  enable = true;
  # An engine for CJK and compose (nothing runs until you set this):
  inputMethod.enable = true;
  # Pinyin beside the built-in keyboard layouts:
  # inputMethod.addons = with pkgs.qt6Packages; [ fcitx5-chinese-addons ];
};
```

What runs: fcitx5 as `scoot-input-method.service` in
`scoot-session.target` (so no other desktop starts it), owning the
`input-method-v2` seat the compositor's
[text-input clients](../scoot/protocols.md#input-methods-text-input-v3-input-method-v2)
talk to. The engine holds nothing periodic: no polling, no watchers
— it wakes on protocol events and D-Bus calls only. A look themes
its candidate window (below); without one fcitx5's own theme
stands.

Why fcitx5, measured at the pinned rev (`8ce4ef6`,
`aarch64-linux`: full closures by `nix path-info --closure-size`
against the binary cache):

| Engine | Version | Full closure | Speaks `input-method-v2` | Verdict |
|---|---|---|---|---|
| fcitx5 | 5.1.21 | 437.4 MiB | yes — its wayland frontend binds the seat (proven live on the M2, [below](#trying-it-headless)) | the slot |
| ibus | 1.5.34 | 1.10 GiB | no — it speaks its own D-Bus protocol (what GNOME's compositor integrates with); it never binds the Wayland global | out |
| kime | 3.1.1 | 1.02 GiB | no — Korean-only, and on the v1/virtual-keyboard path, not `input-method-v2` | out |
| uim | 1.9.6 | 432.5 MiB | no — XIM only, so native Wayland clients get nothing | out |

Size alone would leave uim standing (about the same closure as
fcitx5) — the protocol decides it: on a pure-Wayland scoot session
only fcitx5 has anything to say to a native client. The default
package is plain fcitx5 (keyboard layouts, compose and the XIM
server for XWayland, no CJK data); a CJK engine rides
`inputMethod.addons` beside it (pinyin costs another ~1.52 GiB of
closure at the pinned rev — dictionaries are data). The
`fcitx5-with-addons` wrapper (Qt5, Qt6 and GTK input-panel plugins
plus the config GUI) is deliberately not the default: its closure
is wider than ibus, while GTK3/4 and Qt 6.8+ talk to the engine
over text-input with no plugin at all.

The toolkit variables, only where a toolkit still needs them
(checked against fcitx's own [Using Fcitx 5 on
Wayland](https://fcitx-im.org/wiki/Using_Fcitx_5_on_Wayland)):

| Variable | Value | Who still needs it |
|---|---|---|
| `XMODIFIERS` | `@im=fcitx` | X11/XWayland clients (XIM is unchanged under XWayland) |
| `QT_IM_MODULE` | `fcitx` | Qt5 and Qt6 before 6.8 (whose Wayland platform speaks only text-input-v2, which no compositor but KWin offers) |
| `QT_IM_MODULES` | `wayland;fcitx` | Qt 6.7 and later (tries the native text-input first, falls back to the fcitx module — so the candidate popup stays compositor-placed where the toolkit allows it) |
| `GTK_IM_MODULE` | deliberately unset | nobody: with it unset GTK3/4 use their Wayland text-input natively, which is the path the compositor places the popup for |

The variables land in the user-manager environment (so user units
and D-Bus-activated apps inherit them) and in login shells. A value
you set wins over each default. `SDL_IM_MODULE` stays unset too:
SDL2 on Wayland needs none, and its X11 build reads XIM through
`XMODIFIERS`.

The candidate window follows the look: a generated `scoot-look`
classic-UI theme (unselected text in the look's foreground, the
selected row in its accent with the background on top — the AA
pairing the bar is measured against — the input string in the
accent) plus a two-line `classicui.conf` naming it for both the
light and the dark path (fcitx5 follows the portal's dark scheme
under a `prefer-dark` look, which is every look but `music-desk`).
`theme.targets.inputMethod.enable = false` keeps fcitx5's own style
while the rest follows the look. The `Sans` faces in the theme
resolve through fontconfig, so with the theme child's `fonts`
target they follow the look's UI sans.

Adding the languages yourself: open the config GUI
(`fcitx5-config-qt` from `qt6Packages.fcitx5-configtool`) or edit
`~/.config/fcitx5/profile` by hand — add the engine to the group,
e.g. pinyin beside `keyboard-us`:

```ini
[Groups/0/Items/1]
Name=pinyin
Layout=
```

Then trigger the engine (`Ctrl+Space` by default) and type: the
candidate list opens at the cursor, placed by the compositor over
the text-input surface ([protocols](../scoot/protocols.md#input-methods-text-input-v3-input-method-v2)).

Every value is an option, applied on rebuild/switch (the unit
restarts into a new theme; a new engine or addon rewrites
`ExecStart` and restarts it — no re-login):

| Option | Type | Default | Meaning |
|---|---|---|---|
| `desktop.inputMethod.enable` | bool | `false` (opt-in, never with the profile) | run the engine as a user unit |
| `desktop.inputMethod.daemon` | enum (`"fcitx5"`) | `"fcitx5"` | the engine behind `enable` (a future engine widens this enum, the option staying as it is) |
| `desktop.inputMethod.addons` | list of packages | `[ ]` | CJK engines and toolkit plugins beside the core (pinyin, mozc, hangul, the Qt5 input-panel plugin); empty keeps keyboard layouts, compose and XIM with no CJK data |
| `desktop.inputMethod.package` | package or null | fcitx5 (Linux-only: null off Linux) | point at your own engine build (it must speak `input-method-v2`); null with the switch on fails evaluation naming it |
| `desktop.theme.targets.inputMethod.enable` | bool | `true` | theme the candidate window from the look; `false` keeps fcitx5's own style |

```nix title="configuration.nix and home.nix"
programs.scoot.desktop = {
  enable = true;
  inputMethod.enable = true;
  # Japanese and Korean beside pinyin:
  # inputMethod.addons = [ pkgs.fcitx5-mozc pkgs.fcitx5-hangul ];
  # The candidate window keeps fcitx5's own theme:
  # theme.targets.inputMethod.enable = false;
};
```

Trying it headless: enable the slot with pinyin in `addons`,
start a headless session, open foot, trigger the engine and type
`nihao` — the screenshot shows the committed 你好 and, mid-word,
the candidate list at the cursor. The module checks pin this shape
at eval; the [acceptance run](#trying-it-headless) below proves the
binding live on the M2 (full CJK typing stays a manual step until
a headless text-input client ships — see there).

## Trying it headless

Proven live on the M2 against headless scoot (installed `0.1.0`,
IPC protocol 10 —
the change itself is Nix-only, so no compositor behavior moves):
fcitx5 5.1.21 started with the slot's shape (user
unit past the session target, no polling) binds the seat:

- scoot advertises `zwp_input_method_manager_v2` (global, version
  1) and fcitx5 binds it (`wl_registry.bind`, on the wire under
  `WAYLAND_DEBUG=1`);
- fcitx5 loads `waylandim` and creates its classic UI for the
  Wayland display (`classicui.cpp: Created classicui for wayland
  display`).

What this does not yet prove: composing a CJK string through the
engine into a client (no CJK addon or GUI client was fetched on the
shared box — disk sat at 93%). Do that by hand, once, on any box
with a display:

1. Enable the slot with pinyin beside the core:
   `inputMethod.enable = true` and `inputMethod.addons = with
   pkgs.qt6Packages; [ fcitx5-chinese-addons ];`, rebuild/switch.
2. `systemctl --user status scoot-input-method` is active;
   `echo $XMODIFIERS` prints `@im=fcitx`.
3. Open foot, press `Ctrl+Space` (the engine triggers), type
   `nihao`: the candidate list opens at the cursor in the look's
   colors (`scoot-look` theme), `1` commits 你好.
4. Repeat in a GTK app (Text Editor) and a Qt app: both commit
   over text-input with the popup placed by the compositor; an
   XWayland app (e.g. `xterm`) commits over XIM instead.

Troubleshooting, by symptom:

- *Only US-English types, `Ctrl+Space` does nothing.* The engine
  runs but your group holds no CJK engine: the default package
  ships keyboard layouts only. Add the engine to `addons` and to
  the fcitx5 group (config GUI or `~/.config/fcitx5/profile`,
  above), then trigger it. Check the unit first:
  `systemctl --user status scoot-input-method` — and that it
  started with the display (a hand-started session must reach
  `scoot-session.target` with `WAYLAND_DISPLAY` imported,
  [on the desktop page](./index.md#which-sessions-start-the-units)).
- *The candidate list never opens (commits land as plain
  ASCII).* The client is not on the text-input path: an XWayland
  app reads XIM (check `XMODIFIERS` in its environment), a Qt5 app
  reads the fcitx module (check `QT_IM_MODULE`, and that its
  plugin is installed — the Qt5 input-panel plugin rides
  `addons`, not the default package). GTK3/4 and Qt 6.8+ need
  neither: with the variables above they commit over text-input.
- *Candidates open at the wrong place (or blink).* Something set
  `GTK_IM_MODULE=fcitx` globally: GTK then renders the popup
  in-process instead of letting the compositor place it. Unset it
  (this slot never sets it) and keep the toolkit default.
- *The engine runs inside the wrong desktop.* It cannot: the unit
  is wanted by `scoot-session.target`, which only a scoot session
  reaches — `systemctl --user status scoot-input-method` shows the
  scope it started in.
- *Two engines fight over the seat.* Only one client owns
  `input-method-v2` per seat: turn the other one off (IBus, another
  fcitx5 from a second session, or a desktop portal's own
  daemon) — whichever stays owns every text-input client.
- *Qt6 routes through the fcitx module instead of text-input.*
  Where `QT_IM_MODULE` is read first the module path
  still commits, and
  `QT_IM_MODULES="wayland;fcitx"` prefers the native path on Qt
  6.7+. Unset `QT_IM_MODULE` for that app alone if you want the
  compositor-placed popup unconditionally and never run Qt5 there.
