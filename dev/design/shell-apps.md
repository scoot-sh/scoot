# The rest of the shell: scootnotify, scootlaunch and scootgreet

Design pass, 2026-10-12. Nothing here is built; this is the plan the
tickets under scootbar's M7 are cut from. Three apps share one foundation,
so what they share lives here and each app's own design is a separate file:

- [scootnotify.md](scootnotify.md): the notification daemon (replaces mako).
- [scootlaunch.md](scootlaunch.md): the launcher and dmenu picker (replaces fuzzel).
- [scootgreet.md](scootgreet.md): the login greeter (replaces ReGreet).

This tree (`dev/`) is maintainer material and is never published
([`dev/README.md`](../README.md)). A design is neither a dated measurement
(`benches/`), a spike record (`spikes/`) nor a survey (`research/`), so it gets
its own directory. Once an app ships, its user reference goes to the site
(`/scootnotify/`, `/scootlaunch/`, a greeter section) per
[`site/OUTLINE.md`](../../site/OUTLINE.md) "Coming apps", and these files stay
as the design record.

Every claim cites a file and line in this tree, or a spec section with its URL,
as of `main` at `13173a7`. Where something could not be checked, it says so.

## Contents

1. [Names](#names)
2. [Why build any of them](#why-build-any-of-them)
3. [What each app needs from the shared stack](#what-each-app-needs-from-the-shared-stack)
4. [The shared crates: `scootui` and `scoot-dbus`](#the-shared-crates-scootui-and-scoot-dbus)
5. [Keyboard and text entry](#keyboard-and-text-entry)
6. [Text: shaping, wrapping, emoji](#text-shaping-wrapping-emoji)
7. [Theme tokens, Stylix and looks](#theme-tokens-stylix-and-looks)
8. [Live look swap](#live-look-swap)
9. [Nix modules and the desktop profile switch](#nix-modules-and-the-desktop-profile-switch)
10. [Without Nix](#without-nix)
11. [Packaging and CI](#packaging-and-ci)
12. [Testing that all three share](#testing-that-all-three-share)
13. [Start order across the three apps](#start-order-across-the-three-apps)
14. [The tickets](#the-tickets)
15. [Decisions needed](#decisions-needed)
16. [Rejected alternatives](#rejected-alternatives)

## Names

- **`scootnotify`**, settled. It appears in CLAUDE.md's commit-scope list
  (`CLAUDE.md`, Commits and CI: "`scootbar` (later `scootnotify`, `scootlaunch`)"),
  in the pointer entry `docs/scootbar/backlog/scootnotify.md`, and on the site
  (`site/src/content/docs/start/compose.mdx:22`).
- **`scootlaunch`**, not `scootlauncher`. The name was settled on 2026-09-29
  over `scootmenu` (`docs/scootbar/backlog/launcher.md:16`), and it is already
  load-bearing in shipped text. CLAUDE.md lists it as a scope, and scootbar's
  own docs use `exec = ["scootlaunch"]` as the example button
  (`crates/scootbar/src/config/custom.rs:11`, `crates/scootbar/src/modules/button.rs:8`).
  It also appears in `site/OUTLINE.md` ("`scootlaunch` (launcher …) →
  `/scootlaunch/`") and in the Nix option descriptions that promise the swap
  (`nix/modules/desktop.nix:531-535`). "scootlauncher" in the request is the
  same app. Renaming would touch all of these for no gain, and a verb matches
  the family (below).
- **`scootgreet`** (recommended; listed under [Decisions needed](#decisions-needed)).
  The family names what the binary *does*: `scootlaunch`, `scootnotify`, and
  the future `scootlock` (`site/OUTLINE.md`, Coming apps). In the greetd
  ecosystem the convention is a `-greet` suffix: `agreety`, `gtkgreet`,
  `tuigreet`, `ReGreet`. `scootgreeter` is a noun, like `scootbar`/`scootbg`,
  but those two name *things on screen* (a bar, a background), and the
  newer apps use verbs. The binary, crate, Cargo package, Nix attribute and
  `/etc/scoot/greet.toml` all use the one name.

## Why build any of them

The project's position, from CLAUDE.md: daily-drivable *and* computer-use
capable, "incredibly lightweight, fast and beautiful", with standard
protocols first. Today all three slots are filled by good third-party tools
wrapped by the desktop profile, and each wrapper already promises a native
replacement behind the same option names. The paved-path contract is "replaces a slot *without changing the user's config*
— same option names, same binds, daemon name as the only visible change"
(`docs/backlog/resolved/desktop-paved-path-done.md:93-97`).

Measured baselines (all aarch64, Asahi M2 or dev VM):

| Slot | Today | Closure | Marginal over the profile | RSS | Other |
|---|---|---|---|---|---|
| Notifications | lean mako + `scoot-notify-sync` (busctl + jq feed) | 183.4 MiB | 53.3 MiB | 9.3 MB empty, plus an 11.4 MB feed tree | 0 wakeups/60 s idle (`desktop-notifications-done.md`, Decisions) |
| Launcher | fuzzel via `scoot-launcher` | 152.4 MiB | 41.7 MiB | ~22 MB open, 0 closed | first frame 57/50/52 ms (`desktop-launcher-done.md`, Resolution) |
| Greeter | ReGreet 0.5.0 (GTK4, GStreamer, AccountsService) in `dbus-run-session cage` | **not measured** | **not measured** | **not measured** | nixpkgs `regreet` build inputs: `accountsservice dbus glib gtk4 gst_all_1.{gstreamer,gst-plugins-good,gst-plugins-base} pango librsvg libseccomp` |

For comparison, scootbar's own idle RSS is 3.6 to 4.3 MB depending on the modules
placed (`docs/scootbar/backlog/lightest.md`, M6 tables). The D-Bus spike's
dependency-free name owner ran in 1.9 MB (`dev/spikes/scootbar/dbus-client.md`, Results).

What all three gain, beyond the per-app cases in their own docs:

- **One look, drawn one way.** The same text renderer, color tokens, corner
  tables and opacity handling as the bar, and one Stylix mapping across all of them.
  Today the look reaches mako as four config leaves plus a critical ring
  (`nix/modules/notifications-home.nix:206-235`), reaches fuzzel as seven CLI
  flags that beat the user's own `fuzzel.ini` for those colors
  (`nix/modules/fuzzel-theme.nix`, and `site/src/content/docs/desktop/index.md:757-762`),
  and reaches ReGreet as a hand-written CSS sheet per look. Those sheets need a
  CI gate because one bare `frame {}` rule draws an empty pill above Reboot/Power Off
  (`scripts/check-regreet-css-scope.sh`; `site/src/content/docs/scoot/theming.md:108-125`).
- **Agent-drivable by construction.** Each app gets a JSON interface an agent
  can use without OCR: notification list, invoke and subscribe, launcher
  list and run, and a scriptable login for VM tests (today done with an ad-hoc
  greetd IPC script).
- **Fewer moving parts.** No shell-script feed with `busctl` and `jq`
  (`notifications-home.nix:58-198`), no GTK4 stack on the login screen, and no third-party
  config dialect to track.

What all three lose, said plainly: mature, widely tested code; text rendering
through pango and HarfBuzz, which our stack does not match (see
[Text](#text-shaping-wrapping-emoji)); and the existing user docs and config
knowledge for mako, fuzzel and ReGreet. Each app's doc has a **go/no-go gate**
measured by a first spike or prototype. Keep wrapping if the gate fails. Because of the
paved-path contract, keeping the wrapper costs nothing.

## What each app needs from the shared stack

What exists today is in scootbar, which is binary-only (no `lib.rs`;
`crates/scootbar/src/main.rs:15` `#![forbid(unsafe_code)]`):

| Capability | Exists in scootbar | scootnotify | scootlaunch | scootgreet |
|---|---|---|---|---|
| Canvas, span fills, rounded corners, premultiplied blending | yes (`src/paint.rs`) | yes | yes | yes |
| Single-line text, glyph cache, fallback fonts, ellipsis | yes (`src/text.rs`, `src/font.rs`, `modules/ellipsis.rs`) | yes | yes | yes |
| Multi-line wrap | tooltips only, at most 6 lines (`src/popup/wrap.rs:9,17`) | yes (body) | no | messages (PAM info text) |
| Color tokens | six, `#rrggbb` only (`src/theme.rs:17-32`, `src/color.rs:14-47`) | + border, link | + selection, match | + selection, error |
| Layer surface per output, hotplug, fractional scale | yes (`src/outputs.rs`, `src/density.rs`, `src/daemon/surfaces.rs`) | overlay, on demand | overlay, exclusive keyboard | overlay or full-screen, exclusive keyboard |
| shm double buffer, damage | yes (`src/daemon/canvas.rs`, `SLOTS = 2` at :56) | yes | yes | yes |
| Pointer input | yes (`src/pointer.rs`) | yes | yes | yes |
| Keyboard keymap, modifiers, repeat | **no**: the keymap fd is dropped unread and raw evdev codes are matched (`src/daemon/popup/events.rs:30-38,378`), `wl_seat` is bound at v9 or lower (`src/daemon/binds.rs:260-262`) | no (keybinds go through scoot) | **yes** | **yes** |
| Text entry widget, caret, password mask | **no** | no | **yes** | **yes** |
| List longer than 16 rows | **no**: `MAX_WIDGETS = 16` (`src/popup/mod.rs:83`) | no | **yes** (virtualized) | users and sessions (short) |
| PNG icons, hicolor lookup | yes, hicolor `apps/` only (`src/modules/tray/theme.rs:13-19,117-128`) | yes (`app_icon`, `image-path`) | yes, with theme inheritance | avatars (PNG files) |
| SVG icons | **no**: path data only (`src/icon/path.rs`) | not v1 | **yes, app icons are often SVG-only** | no |
| D-Bus client | yes, and it can already *serve*: `MethodCall` events, `reply_return`, `reply_error`, `signal` (`src/dbus/conn.rs:293,506,522,566`) | **server** of `org.freedesktop.Notifications` | client: `systemd1` transient scopes | client: `login1` power, AccountsService (optional) |
| Control socket (line JSON), `msg reload` | yes (`src/control/`) | yes | no (no daemon) | **no, by design** (pre-login) |
| Fuzz harness, corpus replay on stable | yes (`crates/scootbar/fuzz`, six targets) | Notify body | desktop entries, Exec | greetd frames |

## The shared crates: `scootui` and `scoot-dbus`

The [extract-scootui](../../docs/scootbar/backlog/extract-scootui.md) entry
waits for a second consumer before cutting the seam ("so the seam comes from real
use rather than guesswork"). scootnotify is that consumer, and these plans
define it. Two crates, not one:

### `scootui` (library, `crates/scootui`)

Pure drawing plus the Wayland client scaffolding, behind Cargo features so that
a consumer pays only for what it links. This follows the guard rail in
extract-scootui: "Feature-gate the parts a consumer does not use".

| Feature | Moves from scootbar | New |
|---|---|---|
| (always) | `paint.rs`, `color.rs`, `theme.rs` (tokens grow: below), `text.rs`, `font.rs`, `modules/ellipsis.rs`, `testfont.rs` (test-support), `snapshots.rs` (test-support), `print.rs` | `#aarrggbb` parsing; a token map that is not hard-wired to the bar's six |
| `widgets` | `popup/{mod,layout,interact,paint,hover,wrap}.rs` (the pure half; `extract-scootui.md`, "Popups") | `Activate` carries an owned id instead of `&'static str`; a virtualized list (row count unbounded, drawing bounded by the visible rows); a text-entry widget; wrapped paragraphs past 6 lines with a line cap the caller sets |
| `icons` | `icon/{mod,path,raster,cache,sample}.rs`; `icon/image.rs` behind `png` | icon-theme lookup with `index.theme` inheritance (today hicolor-only, `modules/tray/theme.rs:13-19`); optional `svg` (below) |
| `wayland` | `outputs.rs`, `density.rs`, `region.rs`, `daemon/surfaces.rs`, `daemon/canvas.rs`, the Wayland half of the `poll` loop (`daemon/mod.rs`) | a layer-surface lifecycle that is *on demand* (created when there is something to show, destroyed when empty, with no surface held at idle) beside the bar's one-per-output |
| `keyboard` | none | keymap, modifiers, repeat, keysym to text ([below](#keyboard-and-text-entry)) |
| `control` | `control/{framing,paths,claim,conn}.rs` and the bounds (`MAX_CONNECTIONS`, `MAX_SUBSCRIBERS`, the write-stall deadline; `control/mod.rs:57-65`) | a socket name that is a parameter instead of fixed to `scootbar-…` |

What stays in scootbar: modules, `bar.rs`, `layout.rs` (the three sections),
`render.rs` (the bar's per-module repaint, which is bar-shaped), config,
CLI, the popup's Wayland half (`daemon/popup/`, which is the bar's `xdg_popup` on its
own layer surface), the pointer bindings and actions.

`scootbg-mem` stays the one crate with `unsafe` code. The extraction adds the
memfd name parameter that extract-scootui asks for (today every buffer is named
`scootbg-wallpaper`, `crates/scootbg-mem/src/shm.rs:173`).

**scootbg is not moved onto `scootui`** in this plan. It has a release gate
(`docs/scootbg/backlog/lightest.md`), and extract-scootui says to rerun its
benchmark or "leave it on its own copy and say so". Leaving it costs a second copy of
`outputs.rs`/`density.rs`/`surfaces.rs`, which exists today anyway. A
later ticket can move it once two apps prove the crate stable.

**Risks of the extraction, and the guards.** Moving about 6,000 lines out of a
binary crate that holds a published ratchet:

- *Behavior drift.* The move is mechanical: `git mv` plus `pub` and paths, no
  logic edits in the same PR. scootbar's snapshot tests (`src/snapshots/`, PGM/PPM
  goldens) and its zero-allocation tests (`count_allocations == 0` at
  `render/tests/icons.rs:318`, `popup/tests.rs:825` and others) must pass
  unchanged, with no snapshot re-blessed.
- *Ratchet regression.* Cross-crate inlining changes code layout. With
  `lto = "fat"`, `codegen-units = 1` (root `Cargo.toml:21-26`) the effect should be
  noise, but rule 1 of the [ratchet](../../docs/scootbar/backlog/lightest.md) says
  measure: stripped size, `.text`, idle RSS/PSS, wakeups, fds, against `main`, using
  the M6 method.
- *Fuzz targets* compile scootbar sources by `#[path]`
  (`crates/scootbar/fuzz/Cargo.toml:28-65`). They move with their code, keep their
  corpora, and CI's run budgets stay as they are (`.github/workflows/ci.yml:1412-1450`).
- *`forbid(unsafe_code)`* holds in `scootui` as in scootbar.

### `scoot-dbus` (library, `crates/scoot-dbus`)

The hand-rolled client (`src/dbus/{conn,proto,link,fuzz}.rs`, about 3,600 lines with
its tests) moves to its own crate, because three apps need it and none of that need is
drawing: scootnotify serves `org.freedesktop.Notifications`, scootlaunch calls
`org.freedesktop.systemd1`, and scootgreet calls `org.freedesktop.login1`. The
name follows `scoot-ipc`: a wire-protocol library, hyphenated. The
MPRIS and BlueZ readers (`mpris.rs`, `bluez.rs`) stay in scootbar.

What it needs that it lacks, each added by the first consumer that needs it
(not in the move PR):

- An error reply with a message body. `reply_error` sends a name only
  (`conn.rs:522`), and `org.freedesktop.Notifications.CloseNotification`
  answers an unknown id with "an empty D-Bus Error message", so that call is
  covered. For limits and bad arguments, a text body helps the client's user.
  (scootnotify)
- **A reply for an over-cap method call.** Today a message past `MAX_MESSAGE = 1 MiB`
  (`proto.rs:51`) is skipped. If it was a method call, the caller waits for its
  own timeout: libnotify's default is 25 s, during which `notify-send` hangs. The
  header is read before the body is discarded, so the client can answer
  `org.freedesktop.DBus.Error.LimitsExceeded` to the serial. (scootnotify; verify against
  `conn.rs`'s discard path before building on this.)
- A per-connection message cap, so scootnotify can raise its own to 4 MiB
  ([why](scootnotify.md#security-untrusted-content)) without raising scootbar's.
- No fd passing, no big-endian, no non-unix transports. None of the three needs them.

The `link` lifecycle (`link.rs`: inotify on the socket directory, redial,
three quick deaths then 30 s) serves all three as it is. On reconnect, the
consumer's `start` re-requests its name and re-adds its match rules
(`link.rs:123-129`), which scootnotify must also do.

## Keyboard and text entry

scootlaunch and scootgreet type text, and scootbar never has. This is the
largest new piece of the shared stack and the one most likely to go wrong for
real users, because a non-US keyboard that types the wrong characters into a
password field locks the user out.

What Wayland gives a client: `wl_keyboard.keymap` (an fd with an XKB text
keymap, format `xkb_v1`), `key` (evdev codes), `modifiers` (depressed, latched,
locked, group) and `repeat_info` (rate and delay, from `wl_seat` v4). Repeat is the client's own
job, and on `wl_seat` v10 the compositor sends `repeated` key events. The keymap is a full
XKB description, and parsing it is the hard part.

Options, to be decided by a spike (`scootui-keyboard-spike`):

| Option | For | Against |
|---|---|---|
| **`kbvm`** (pure Rust XKB implementation; 0.2.0, MIT OR Apache-2.0, crates.io, checked 2026-10-12) | No C library. Keeps the family's "links only libc, libm, libgcc_s" property (scootbar's measurement, `lightest.md` M5/M6) | Young (0.2.0, about 22k downloads); compose and dead-key coverage unknown; one maintainer |
| **libxkbcommon** via the `xkbcommon` crate (0.9.0, MIT; already in `Cargo.lock` because scoot links it through Smithay) | The reference implementation, which every compositor and toolkit uses; compose tables; zero marginal closure on a scoot desktop | A C shared library in a binary family that has none; FFI means `unsafe` (it lives in the crate, not in `scootui`, but it ends `forbid(unsafe_code)` for the consumer or forces a wrapper crate) |
| Hand-rolled | Smallest | Wrong for any non-trivial layout; rejected |

**The spike measures**, against the keymap scoot actually sends (scoot builds
it from `XKB_DEFAULT_*`, `crates/scoot/src/compositor/state.rs:1282-1287`):
correctness on `us`, `us(intl)` dead keys, `de`, `fr` (AZERTY, shifted digits),
`dvorak`, a two-group `us,ru` with `grp:alt_shift_toggle`, and Compose
sequences; keymap compile time; stripped size and RSS added. **Prior,
for the spike to confirm or overturn:** `kbvm` if it passes the layout set
including dead keys and costs under 300 KB; otherwise `xkbcommon`, wrapped in
a small `scootui-xkb` crate that holds the `unsafe` code. Compose (the
`XCompose` file) is required for scootgreet only if a password can contain a
composed character. The spike reports whether either option handles it.

**IME.** scoot implements `text-input-v3` and `input-method-v2`
(`site/src/content/docs/scoot/protocols.md`, "text-input"), and focus follows the
keyboard, layer surfaces included. A launcher with CJK search wants it. **v1 is
non-IME**, as the launcher pointer already said ("no IME at first, stated";
`launcher.md:24-25`). A follow-up ticket adds `text-input-v3` to the entry
widget. The greeter never enables IME for the password field: candidate popups
would show the password.

**The text-entry widget** (in `scootui` `widgets`), required by scootlaunch and
scootgreet:

- One line, a byte cap the caller sets (launcher 512 B, password 1 KiB), UTF-8
  aware caret (grapheme clusters approximated by `char` boundaries in v1, stated),
  and selection limited to select-all plus delete.
- Editing keys: Left/Right, Home/End, Ctrl+Left/Right word moves, BackSpace, Delete,
  Ctrl+W, Ctrl+U, Ctrl+A select-all. Clipboard paste (`wl_data_device`) in the
  launcher only. It is never offered on a password field, which also refuses copy.
- **Password mode**: draws a fixed glyph per character (or a fixed-width
  dot row that does not leak length, as an option), and keeps the buffer in a
  `zeroize`-on-drop fixed allocation (`zeroize` 1.9, Apache-2.0 OR MIT). The
  allocation is never resized, so no copy of the secret is left in freed heap.
  scootgreet's doc covers the rest of the secret's path.
- Key repeat: a `timerfd` armed on press of a repeatable key and disarmed on release
  or focus loss. Rate and delay come from `repeat_info`; a rate of 0 means no repeat.
  At idle it costs zero wakeups (no timer exists).
- Focus ring: the entry draws a 2-device-pixel ring in the `accent` token when
  focused.
- Lock-key state from `modifiers` (Caps Lock and Num Lock bits) is exposed for the
  greeter's warning.

## Text: shaping, wrapping, emoji

The bar draws one glyph per `char` with no shaping, kerning, bidi or color emoji
(`crates/scootbar/src/text.rs:1-21`), with a primary font and at most two
fallbacks (`text.rs:63`). For a bar label that is fine. A notification body is
arbitrary user content in any language, so this matters more there:

- Latin, Greek, Cyrillic and CJK draw acceptably without shaping (no kerning,
  which is visible but not wrong). Arabic, Hebrew (bidi), and Indic scripts draw
  **wrong**: unjoined, reversed, broken clusters.
- Color emoji (CBDT/COLR) do not draw. With a monochrome outline emoji font
  as a fallback they draw as outlines.

**Spike `scootui-text-shaping-spike`**: put `rustybuzz` (0.20.1, MIT, a
HarfBuzz port; it reads fonts through `ttf-parser`, the same parser
`ab_glyph` already pulls in via `owned_ttf_parser` 0.25.1, so the parser is
shared — verify in the spike) behind a `shaping` feature. Also measure
`unicode-bidi` (MIT OR Apache-2.0) for RTL runs. Measure binary size, RSS, and
shape time per notification body (once per notification, cached as positioned
glyphs; never per frame). **Gate for scootnotify v1:** if shaping costs under
about 250 KB of `.text` and the per-body cost is under 1 ms, it ships in scootnotify's
default build. Otherwise v1 ships without it and the docs say which scripts
draw wrong. scootlaunch has the same need for app names (localized `Name[ar]`).
scootgreet shows user names and session names, where it matters least.

Line breaking for bodies: break at spaces and after CJK characters, with a hard break for
over-long words (what `popup/wrap.rs` does, minus its 6-line cap). The
`unicode-linebreak` crate is Apache-2.0, which is allowed by `deny.toml`, but at about
15 KB of tables it is not needed for v1.

Fallback fonts: raise `MAX_FALLBACKS` from 2 to 4 in `scootui` (CJK + emoji
+ symbols + the user's), as a per-consumer constant, so the bar's bound is
unchanged.

## Theme tokens, Stylix and looks

### One token set

All three apps read one token vocabulary, so a look or a Stylix palette maps
once. These are the bar's six tokens (`crates/scootbar/src/theme.rs:17-32`) plus
three that menus and cards need:

| Token | Role | Stylix slot | Why that slot (base16 styling guide: `tinted-theming/home` `styling.md`) | Look source (`nix/modules/desktop.nix` registry) |
|---|---|---|---|---|
| `background` | surface fill | `base00` | "Default Background" | `barColors.background` |
| `foreground` | primary text | `base05` | "Default Foreground, Caret" | `barColors.foreground` |
| `dim` | secondary text (app name, time, placeholder), low-urgency border | `base03` | "Comments, Invisibles"; scootbar's `dim` is `base03` (`nix/modules/scootbar.nix:479-496`), and so is Stylix's mako low-urgency border (below) | `barColors.dim` |
| `accent` | border, focus ring, caret, normal-urgency border, links | `base0D` | "Functions, Headings": blue. scootbar moved its Stylix accent from `base0A` to `base0D` on purpose so "highlights and window ring agree" (`scootbar.nix:102-110`), and Stylix's own mako and fuzzel borders are `base0D` | `appearance.focus_ring_active_color` for borders (as mako and fuzzel get today), `barColors.accent` otherwise |
| `hover` | pointer hover | `base0D` | as the bar | `barColors.hover or barColors.accent` |
| `urgent` | critical border, errors, Caps Lock warning | `base08` | "Red"; scootbar and Stylix's mako critical border both use `base08` | `barColors.urgent` |
| `selection` | selected row fill | `base02` | "Selection Background", and Stylix's fuzzel `selection` | `barColors.accent` (today's fuzzel look flag, `fuzzel-theme.nix:18`) |
| `selection-text` | text on the selected row | `base05` | Stylix's fuzzel `selection-text` | `barColors.background` (`fuzzel-theme.nix:19`) |
| `match` | matched characters in the launcher | `base0A` | "Search Text Background", and Stylix's fuzzel `match` | `barColors.hover or barColors.accent` (`fuzzel-theme.nix:20`) |

Stylix values verified at the rev the repo's tests pin (`fb28acd59e2ac1984ec84fa496599d6b4bf3e690`,
`scripts/scootbar-stylix-test.sh:36`). Stylix's mako target:
normal `border-color = base0D`, low `base03`, critical `base08`,
`text-color = base05`, `background-color = base00 + opacity.popups`,
`progress-color = over base02`. Its fuzzel target: `background = base00 + opacity.popups`,
`text = base05`, `placeholder = base03`, `match = base0A`,
`selection = base02`, `selection-text = base05`, `border = base0D`.

**The selection slot is the one deliberate split.** With Stylix, the selected row
is `base02` with `base05` text, the base16 meaning and what every
Stylix-themed menu does. With a look, it is the accent fill with background-colored
text, which is what the look's fuzzel shows today. That keeps every look's current
screenshots true and keeps Stylix users' menus consistent with their other Stylix apps. The
alternative (accent fill everywhere) was rejected: on a base16 scheme `base0D` behind
`base00` text can fail contrast in light schemes, and `base02` is designed for that use.

### Fonts and sizes

- **Family**: Stylix `fonts.sansSerif` (`{ name, package }`), resolved to a font
  *file* by scootbar's `fontFile` helper (`nix/modules/scootbar.nix:62-95`), which
  is lifted into a shared Nix helper (`nix/modules/font-file.nix`) so that all four
  modules resolve a family the same way and share its documented limits
  (variable fonts and `.ttc` fall back to DejaVu with a warning).
- **Size**: scootnotify and scootlaunch take `stylix.fonts.sizes.popups` ("notifications,
  popups, and other overlay elements of the desktop", default = `desktop` = 10 pt;
  `stylix/fonts.nix:103-105` at the pinned rev), which is exactly what Stylix's own mako and fuzzel
  targets use. scootgreet takes `sizes.applications` (12 pt by default): the login
  screen is read from further back and today's greeter size is 12 pt
  (`nix/modules/nixos.nix:2456-2460`, "the measured greeter size").
- **Conversion**: points to logical pixels with the bar's formula,
  `px = clamp(1, 256, (pt * 4 + 1) / 3)` (`scootbar.nix:97-100`): 10 pt = 13 px,
  12 pt = 16 px.
- **Look**: `fonts.ui` (the bar's face, a Nerd Font Propo for glyph icons) for
  scootnotify and scootlaunch, as the bar uses (`scootbar.nix:391-409`). For
  scootgreet it is `fonts.sans`, the face ReGreet gets today (`nixos.nix:2456-2460`).

### Opacity

Stylix `opacity.popups` (0.0 to 1.0, default 1.0; `stylix/opacity.nix:18-21` at
the pinned rev) becomes scootnotify's and scootlaunch's background alpha, as
Stylix's mako and fuzzel targets do. scootbar does not read Stylix opacity today. That
is left alone here and noted for the bar's owner. scootgreet is opaque: the
greeter draws over its own wallpaper, and translucency would need a
wallpaper behind it, which it owns.

### Precedence (per leaf, as scootbar)

User > Stylix > look > plain default, implemented per leaf exactly as
`scootbar.nix:7-28` documents. The rungs are: user at priority 100 (plain
assignment), Stylix at `lib.mkDefault` (1000), look at `lib.mkOptionDefault`
(1500), and the module's own plain default at `lib.mkOverride 1600` (only where the
look also sets the same key, as the bar's `font`). Each value is set on
`settings.colors.accent`, never on `settings.colors`, so a user's single key
keeps the rest. Stylix presence is detected as in `scootbar.nix:40-44`
(`config.lib ? stylix && (config.stylix.enable or false)`), with a per-app
`programs.<app>.stylix.enable` opt-out. There is no upstream `stylix.targets.<app>`:
that is an upstream change, and no agent opens upstream PRs (CLAUDE.md).

### Look files

Each look directory `docs/examples/<look>/` gains a `notify.toml` and a
`launch.toml` beside its `bar.toml`, and the looks that ship `regreet.css`
gain `greet.toml`. These are complete config files in each app's own format. They
are what `install.sh --look` copies (today it copies `scoot.toml`, `bar.toml`,
the scripts and the wallpaper, `site/public/install.sh:424-455`). The Nix
profile does **not** apply them wholesale, exactly as it does not apply `bar.toml`'s
geometry today. It maps the registry's roles (`barColors`, `appearance`,
`fonts`, `isDark`, `wallpaper`) onto each app's color and font leaves.
Geometry (radius, width, anchor) stays the app's default unless the user sets
it. The registry needs **no new fields**. Every token above derives from fields it
already has, so a new look keeps working with no edits for these apps.
`look-moonrise-done.md:46-48` says looks should be "role-named colors so the
coming registry absorbs it without rework", and this respects that.

## Live look swap

No live-look-swap design exists in this tree or on the remote as of
2026-10-12. There is no `docs/live-look-swap-design` branch and no open PR
(`gh pr list`), and a grep of `docs/ dev/ nix/ site/ ROADMAP.md` finds no
mention. The nearest mechanisms today are rebuild-time: the look is an
eval-time enum, and units restart through `X-Restart-Triggers`
(`nix/modules/scootbar-home.nix:49`; `notifications-home.nix:326`). So this
plan sets a contract any swap mechanism can call, and changes nothing it
might choose:

1. **Each daemon re-themes in place on `<app> msg reload`**: the same verb as
   `scootbar msg reload` and `scoot msg reload`. A reload re-reads the config
   file, re-maps fonts (as scootbar's reload does, `crates/scootbar/src/daemon/respond.rs:259-286`),
   rebuilds the theme, and repaints what is on screen. State survives:
   notifications on screen, history, DND, ids. A file that fails to parse
   keeps the running config and answers with the error, as scootbar does. Cost
   target: under 5 ms plus a font load. That is cheap enough to call for every
   look change, so a swap tool can call all three without staging.
2. **No SIGHUP.** Catching it needs `unsafe` signal registration, which
   `forbid(unsafe_code)` refuses. This is scootbar's reason, recorded at
   `crates/scootbar/src/daemon/mod.rs:32-34`.
3. **No file watch by default.** An inotify watch on the config costs a
   source and turns a half-written file into a reload. An explicit verb is what scootbar chose,
   and these apps stay consistent with it. A swap tool that wants a
   watch can run `inotifywait … && scootnotify msg reload`.
4. **scootlaunch needs no signal.** It is not resident: every open reads its config,
   so the next open after a swap draws the new look.
5. **scootgreet is out of scope for live swap.** It runs before any user session,
   as the `greeter` user, from `/etc`. "Apply the current look to the login screen"
   is a root-level rebuild step ([scootgreet.md](scootgreet.md#stylix-and-look-integration)).
6. **Under Nix, a config change reloads scootnotify rather than restarting it.** A
   restart throws away what is on screen and the history. Home Manager supports
   `X-Reload-Triggers` (verified in `nix-community/home-manager` `modules/systemd.nix`
   on master, 2026-10-12; **check the rev the tests pin before relying on it**) with
   `ExecReload = scootnotify msg reload`. On NixOS the unit is a *user* unit
   (as `scootbar-nixos.nix` declares the bar's, with `restartTriggers`), and what
   `switch-to-configuration` does to running user units on a config change is
   **not verified here**. `notify-nix` checks it at the pinned nixpkgs before
   choosing `reloadTriggers` (mind the caveat at `nix/modules/nixos.nix:2398-2402`:
   nixpkgs warns on every rebuild when a unit sets both `reloadIfChanged` and
   `reloadTriggers`) or an activation step that runs `scootnotify msg reload`. scootbar keeps its restart, since it holds no state worth keeping.

When the live-look-swap design lands, it chooses the trigger. This contract only
promises that the apps re-theme on `msg reload` without a restart and without
losing state.

## Nix modules and the desktop profile switch

Each app follows the scootbar module layout (`nix/modules/scootbar.nix` plus
`scootbar-home.nix` plus `scootbar-nixos.nix`):

- `nix/modules/<app>.nix`: options (`enable`, `package`, `features`, free-form
  `settings` typed as TOML, `stylix.enable`, `systemd.enable`), the Stylix and look
  rungs, and the rendered `configFile`. It reads the scoot desktop profile
  across the module boundary only by `config.programs.scoot or { }`, as
  `scootbar.nix:116-125` does, so it evaluates without the scoot module.
- `nix/modules/<app>-home.nix` and `<app>-nixos.nix`: file placement and units.
  scootgreet is NixOS-only (a login screen is system state).
- Flake: `homeModules.<app>` and `nixosModules.<app>`, kept out of `default`
  as scootbar's are (`flake.nix:860-899`); `packages.<app>`; `overlays.default`
  gains the package.

**The profile switch, opt-in first.** The slot enums were written to widen:

- `programs.scoot.desktop.notifications.daemon = enum [ "mako" ]`
  (`nix/modules/desktop.nix:441-453`: "a future scootnotify widens this enum, the
  option and the bar module staying as they are") becomes `[ "mako" "scootnotify" ]`,
  with the default staying `"mako"`.
- `programs.scoot.desktop.launcher.daemon = enum [ "fuzzel" ]` (`desktop.nix:546-571`)
  becomes `[ "fuzzel" "scootlaunch" ]`, default `"fuzzel"`.
- `programs.scoot.greeter` gains `daemon = enum [ "regreet" "scootgreet" ]`,
  default `"regreet"`.

The option names, binds, bar module id (`notifications`) and its icons
(`desktop.nix:492-529`) do not change. The `scoot-launcher` wrapper keeps its
name and passes `"$@"` through, so every call site (keymap, pickers, bar) is
unchanged.

**Criteria for flipping a default** (each needs the maintainer's ruling, as
the ratchet's waivers do):

1. The app's go/no-go gate passed and is published.
2. Every row of its resource ratchet is at or under the incumbent's beyond
   noise, or carries a written waiver.
3. Two weeks of daily use on the M2 by the maintainer with no open
   high-priority bug against it.
4. A real-login proof on the M2 per look (screenshots), and for scootgreet a
   QEMU boot test in `scoot-iso` (its `scripts/qemu-test.sh` is today the only
   end-to-end greeter boot).
5. The incumbent stays one option away for at least one release after the flip.

## Without Nix

- Config: `$XDG_CONFIG_HOME/scoot/notify.toml` and `…/scoot/launch.toml` beside
  `bar.toml` (`crates/scootbar/src/config.rs:156`, the bar's `default_path`),
  `--config PATH` to override, `--check` to validate without a compositor, as
  `scootbar daemon --check` does. scootgreet: `/etc/scoot/greet.toml`, owned by root and
  readable by the `greeter` user.
- Units: scootnotify ships a user unit (`Type=dbus`,
  `BusName=org.freedesktop.Notifications`, `ExecReload=scootnotify msg reload`)
  and a D-Bus activation file `org.freedesktop.Notifications.service` with
  `SystemdService=scootnotify.service`. Install docs say to remove or mask mako's
  or dunst's activation file first ([why](scootnotify.md#name-ownership-and-coexistence)).
  scootlaunch ships no unit. scootgreet ships a `greetd` config snippet, not a
  unit.
- `site/public/install.sh` gains `--notify` and `--launch` (opt-in, mirroring how
  `--no-bar` is opt-out for the bar) and copies the look's `notify.toml` and
  `launch.toml` with `--look`. It never installs scootgreet: replacing a login
  screen from a curl pipe is the kind of hard-to-reverse change CLAUDE.md asks
  explicit confirmation for. The greeter's install docs are a manual
  `/etc/greetd/config.toml` edit, with the VT-switch recovery path written next to it.

## Packaging and CI

- **CI path filters** (CLAUDE.md: "Every new package or component adds its path
  filter to the classify job … in the same PR that adds it"): each crate's first
  PR adds a classify output (`scootui`, `scootdbus`, `scootnotify`,
  `scootlaunch`, `scootgreet`), arms for `crates/<crate>/Cargo.toml` (with
  `scoot=true version=true`, as scootbar's does, because the flake reads the
  description) and `crates/<crate>/*` before the catch-all, and a job gated on its
  output (`.github/workflows/ci.yml:194-355` holds the pattern). The shared crates fan
  out: a `crates/scootui/*` change sets `scootbar`, `scootnotify`, `scootlaunch`
  and `scootgreet`, and a `crates/scoot-dbus/*` change sets every D-Bus
  consumer. This is the rule CLAUDE.md states: "A change to shared code fans
  out to its dependents and nothing more".
- **Fuzz**: each app's targets join the fuzz job with fixed budgets (scootbar's
  are 1M to 5M runs, `ci.yml:1412-1450`).
- **Distro packages**: deb (`packaging/deb/build-deb.sh:93`'s package loop and the
  `deb-meta.py` tables), rpm (`packaging/rpm/<app>.spec`, with `%check` running
  `--check` on the example config), and Arch (`packaging/arch/<app>{,-git}/`).
  These come in each app's packaging ticket, after the binary is usable, not in its
  first PR.
- **Versioning**: each crate is versioned independently by `scripts/version`
  from Conventional Commits with its scope (CLAUDE.md, Commits and CI). The new
  scopes `scootnotify`, `scootlaunch` and `scootui` are already named in CLAUDE.md.
  `scoot-dbus` and `scootgreet` are not: adding them to CLAUDE.md's scope list is
  a `docs(claude)` line in the first PR that creates each crate.

## Testing that all three share

- **Unit and snapshot tests** in `scootui`, moved with the code and kept at
  zero per-frame allocation (`scootbg_mem::count_allocations`).
- **Headless harness**: scootbar's `tests/common/` (headless scoot and sway
  sessions, a hermetic `XDG_*` sandbox per test, screenshot readers through scoot's
  IPC and `wlr-screencopy`) moves into a `scootui` test-support module so
  each app's integration tests drive the real binary under `scoot --headless`
  and read real pixels. Input comes through scoot's IPC `key`, `type`
  and `pointer` requests, the same path an agent uses. Tests skip without a compositor
  unless `<APP>_REQUIRE_SCOOT=1`, as scootbar's do.
- **Goldens per look**: the five looks times each app's main states, rendered with
  the seven-segment test font for the pixel-exact goldens, plus one real-font
  screenshot per look kept as a review artifact rather than a golden. A real font's
  pixels change with every font-package update, so they are not stable enough for a golden.
- **Resource ratchet**: each app gets a section in its own `lightest`-style entry
  with rows modeled on scootbar's (idle RSS/PSS, wakeups per minute, fds,
  threads, stripped size and `.text`, closure, startup to first frame), measured by
  the M6 method (release builds, `/proc/PID` sampled after 14 s of settling and
  again 60 s later, one run per row, the run-to-run noise stated). Competitors are
  measured in the same harness.
- **Hardware**: the M2 for real-login proofs (CLAUDE.md, Verification and evidence),
  the cache-key rule for evidence (the SHA the run was taken at), and
  the dev VM for everything else.

## Start order across the three apps

Ordered by what unblocks what, and by risk:

1. **Two spikes, in parallel** (both research, neither ships code):
   [`scootui-keyboard-spike`](../../docs/scootbar/backlog/scootui-keyboard-spike.md)
   and [`scootui-text-shaping-spike`](../../docs/scootbar/backlog/scootui-text-shaping-spike.md).
   They decide the two choices that are expensive to reverse.
2. **The extraction**, in three mechanical PRs, each held to scootbar's
   ratchet: `scootui-extract-draw`, then `scoot-dbus-extract`, then
   `scootui-extract-wayland`. Nothing new is added in them.
3. **scootnotify first.** It needs only what exists: the D-Bus server
   role, pointer input, multi-line text, on-demand overlay surfaces. It is the app
   with the clearest gain (two processes and a shell feed replaced by
   one) and its own go/no-go prototype (`notify-prototype`) can start right
   after the draw extraction, before the Wayland extraction lands.
4. **The keyboard and the entry widget** (`scootui-keyboard`,
   `scootui-entry-and-list`), built to the spike's decision while scootnotify is
   in progress, since no scootnotify ticket needs them.
5. **scootlaunch second.** It is the first consumer of keyboard, entry and the
   virtualized list, and a typo in a launcher costs nothing.
6. **scootgreet last.** It has the same widgets plus a security boundary, a login
   that must never be stranded, and a compositor-host question. By then the
   entry widget has been in daily use in the launcher. Its baseline measurement
   (`greet-baseline`) can run at any time and should run early, because nothing
   in this repo has measured ReGreet.

The **riskiest decisions**, and what is prototyped first to retire them: the keyboard
stack (spike 1), shaping (spike 2), whether scootnotify beats mako on the gate
(`notify-prototype`), and for scootgreet whether scoot itself can safely host
the greeter (`greet-scoot-kiosk-spike`, which is not on the critical path:
cage hosts it until that spike says otherwise).

## The tickets

All live in `docs/scootbar/backlog/` (area `scootbar`, milestone `M7`), the
backlog that already holds the three pointer entries this plan turns into
epics. The one compositor-side ticket, scoot's greeter host mode, lives in
`docs/backlog/core/`. `scripts/backlog` knows a fixed set of areas
(`scripts/backlog`, `BACKLOGS`), and adding `scootnotify`, `scootlaunch` and
`scootgreet` areas is a tooling change outside this docs-only PR; see
[Decisions needed](#decisions-needed).

Every ticket's `blocked` names its predecessors by slug, so `scripts/backlog resolve`
unblocks the next one. **The roots are blocked on the maintainer's go-ahead**, so a
`claim --next` swarm does not start building three apps off the back of a
design PR.

Shared:

| Slug | What | Serves | Blocked by |
|---|---|---|---|
| `extract-scootui` (epic, existing) | the shared foundation | both | its children |
| `scootui-keyboard-spike` | kbvm vs xkbcommon, measured | both | go-ahead |
| `scootui-text-shaping-spike` | rustybuzz + bidi cost, measured | daily-drive | go-ahead |
| `scootui-extract-draw` | move pure drawing, widgets, icons into `crates/scootui` | both | go-ahead |
| `scoot-dbus-extract` | move the D-Bus client into `crates/scoot-dbus` | both | `scootui-extract-draw` |
| `scootui-extract-wayland` | move layer-surface, shm, outputs scaffolding; on-demand surfaces | both | `scootui-extract-draw` |
| `scootui-keyboard` | keymap, modifiers, repeat, keysym to text | both | `scootui-keyboard-spike`, `scootui-extract-wayland` |
| `scootui-entry-and-list` | text entry (password mode), virtualized list, focus ring | both | `scootui-keyboard` |

scootnotify (detail in [scootnotify.md](scootnotify.md#tickets)): `scootnotify` (epic),
`notify-prototype`, `notify-server`, `notify-popups`, `notify-control`,
`notify-bar-feed`, `notify-nix`, `notify-packaging`, `notify-default-flip`.

scootlaunch (detail in [scootlaunch.md](scootlaunch.md#tickets)): `launcher` (epic),
`launch-desktop-index`, `launch-match-rank`, `launch-ui`, `launch-dmenu`,
`launch-agent-json`, `launch-windows-mode`, `launch-nix`, `launch-packaging`,
`launch-default-flip`.

scootgreet (detail in [scootgreet.md](scootgreet.md#tickets)): `scootgreet` (epic),
`greet-baseline`, `greet-greetd-client`, `greet-ui`, `greet-nixos`,
`greet-packaging-iso`, `greet-default-flip`, plus `greet-scoot-kiosk-spike` and
`scoot-greeter-mode` (core).

## Decisions needed

Each item names the recommendation. Everything else in these four documents is
decided, with the evidence beside it.

1. **Go-ahead to start, and in what order?** The roots are blocked on it.
   *Recommend:* start the two spikes and `scootui-extract-draw` now, then follow
   [Start order](#start-order-across-the-three-apps). (CLAUDE.md
   says neither of computer use or daily-drive silently wins. These serve both,
   but they are polish over working wrappers, so the maintainer's queue decides.)
2. **The greeter's name.** *Recommend:* `scootgreet` ([Names](#names)).
3. **A backlog area per app.** Today the tickets live in scootbar's M7.
   *Recommend:* yes, once each app's first code PR lands. That is a small
   `scripts/backlog` change (three `BACKLOGS`/`RESOLVED`/`INDEXES` rows plus a
   test) and a `git mv` of open entries, so it should not be a side effect of a
   design PR.
4. **A second shared crate.** *Recommend:* `scoot-dbus` separate from `scootui`
   (three consumers, none of them drawing). The alternative is a `dbus`
   feature inside `scootui`: fewer crates, but a misleading name.
5. **A C library in the shell family**, if the keyboard spike lands on
   libxkbcommon. *Recommend:* accept it, confined to one small wrapper crate,
   if `kbvm` fails the layout set. A launcher or greeter that types the wrong
   character is worse than a dynamic library that every Wayland session already
   has in its closure.
6. **SVG app icons in scootlaunch.** Many apps ship scalable-only icons, and
   `resvg` (0.48.1, Apache-2.0 OR MIT) with `tiny-skia` (BSD-3-Clause) is the
   only sound renderer, though not small. *Recommend:* behind a default-on `svg`
   feature in scootlaunch only, measured in `launch-ui`, with a waiver ruling on
   its size row as scootbar's tray icons got. The fallback is a letter tile.
7. **Shaping in scootnotify's default build**: decided by the spike's gate
   above. It is listed here because if the gate fails, the maintainer chooses
   between shipping without RTL and Indic scripts (documented) and paying the
   size.
8. **Notification history on disk**: *Recommend:* no. History stays in memory
   (OTP codes and message previews should not be written down).
   An opt-in `history-file` can come later if asked for.
9. **scoot as the greeter's host** (a locked-down kiosk mode in scoot) instead of
   cage. *Recommend:* cage hosts scootgreet first, and the kiosk mode is a later,
   opt-in step gated on its spike ([scootgreet.md](scootgreet.md#which-compositor-hosts-it)).

## Rejected alternatives

- **One shell process** (bar, notifications, launcher, greeter in one binary, as
  AGS, Quickshell and DMS do). Rejected by scootbar's settled decision "One
  small daemon per component" (`docs/scootbar/backlog/README.md`, Decisions),
  and the greeter must run as another user anyway.
- **GTK4 or Qt for any of them.** Their closure and RSS are the cost being removed.
- **`zbus`.** It needs an async runtime; the spike measured +1.1 MB stripped, 86
  crates and 4 threads (`dev/spikes/scootbar/dbus-client.md`).
- **`nucleo-matcher` for fuzzy matching.** It is MPL-2.0, which `deny.toml`'s license
  allow-list (MIT, Apache-2.0, ISC, BSD-3-Clause, Unicode-3.0, Zlib, Unlicense,
  0BSD) does not admit. `fuzzy-matcher` (MIT) is small but unmaintained since
  0.3.7, so it is hand-rolled ([scootlaunch.md](scootlaunch.md#matching-and-ranking)).
- **A config watch (inotify) as the reload trigger.** See [Live look swap](#live-look-swap).
- **Moving scootbg onto `scootui` now.** Its release gate makes that its own
  ticket, later.
