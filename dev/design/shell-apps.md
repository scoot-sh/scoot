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
7. [Theme: the live-looks contract, Stylix and looks](#theme-the-live-looks-contract-stylix-and-looks)
8. [Live look swap](#live-look-swap)
9. [Nix modules and the desktop profile switch](#nix-modules-and-the-desktop-profile-switch)
10. [Without Nix](#without-nix)
11. [Packaging and CI](#packaging-and-ci)
12. [Testing that all three share](#testing-that-all-three-share)
13. [Start order across the three apps](#start-order-across-the-three-apps)
14. [The tickets](#the-tickets)
15. [Decisions needed](#decisions-needed)
16. [Deferred](#deferred)
17. [Rejected alternatives](#rejected-alternatives)

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
| Notifications | lean mako + `scoot-notify-sync` (busctl + jq feed) | 183.4 MiB (stock mako: 357.4 MiB) | 53.3 MiB | 9.3 MB empty, plus an 11.4 MB feed tree. Stock mako 1.11.0, idle: 9,424 kB RSS, 7,332 kB PSS, 1 thread, 8 fds | 0 wakeups/60 s idle (`desktop-notifications-done.md`, Decisions; stock re-measured by the #579 review) |
| Launcher | fuzzel via `scoot-launcher` | 152.4 MiB | 41.7 MiB | ~22 MB open with icons, 0 closed. fuzzel 1.14.1 open, empty `$HOME`: 18,944 kB RSS, 15,951 kB PSS, 17 threads | first frame 57/50/52 ms (`desktop-launcher-done.md`, Resolution) |
| Greeter | ReGreet 0.5.0 (GTK4, GStreamer, AccountsService) in `dbus-run-session cage` | ReGreet 956.7 MiB; ReGreet ∪ cage 1,018.5 MiB (307 paths). cage stays the host (380.2 MiB), so **the most scootgreet can remove is 638.4 MiB** (166 paths), before subtracting what the profile already carries. Package unions only: no greetd, dbus or accounts-daemon | not measured (`greet-baseline`) | not measured (`greet-baseline`) | nixpkgs `regreet` build inputs: `accountsservice dbus glib gtk4 gst_all_1.{gstreamer,gst-plugins-good,gst-plugins-base} pango librsvg libseccomp` |

The figures added in the #579 review were measured on the M2 at the pinned nixpkgs
(`8ce4ef6c`): under `scoot --headless` with a fresh `XDG_RUNTIME_DIR` and `HOME`, sampled after
14 s and again 60 s later; closures from `nix path-info -S` against cache.nixos.org.

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
knowledge for mako, fuzzel and ReGreet. Each app's doc has a **go/no-go gate**: an
out-of-workspace prototype that runs **before any extraction**, so a no costs no sunk
refactor. Each gate's memory and closure rows are close to certain to pass. What can really
fail is text quality, typing correctness and, for the greeter, greetd's session lifecycle, so
the gates test those. Keep wrapping if a gate fails. Because of the paved-path contract,
keeping the wrapper costs nothing.

## What each app needs from the shared stack

What exists today is in scootbar, which is binary-only (no `lib.rs`;
`crates/scootbar/src/main.rs:15` `#![forbid(unsafe_code)]`):

| Capability | Exists in scootbar | scootnotify | scootlaunch | scootgreet |
|---|---|---|---|---|
| Canvas, span fills, rounded corners, premultiplied blending | yes (`src/paint.rs`) | yes | yes | yes |
| Single-line text, glyph cache, fallback fonts, ellipsis | yes (`src/text.rs`, `src/font.rs`, `modules/ellipsis.rs`) | yes | yes | yes |
| Multi-line wrap | tooltips only, at most 6 lines (`src/popup/wrap.rs:9,17`) | yes (body) | no | messages (PAM info text) |
| Color tokens | six, `#rrggbb` only (`src/theme.rs:17-32`, `src/color.rs:14-47`) | the contract's roles (+ `surface`, `focus`) | + `surface`, `focus`, `accent-fg` | + `surface`, `focus`, `accent-fg` |
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
| (always) | `paint.rs`, `color.rs`, `theme.rs` (tokens grow: below), `text.rs`, `font.rs`, `modules/ellipsis.rs`, `testfont.rs` (test-support), `snapshots.rs` (test-support) | `#aarrggbb` parsing; a token map that is not hard-wired to the bar's six |
| `widgets` | `popup/{mod,layout,interact,paint,hover,wrap}.rs` (the pure half; `extract-scootui.md`, "Popups") | `Activate` carries an owned id instead of `&'static str`; a virtualized list (row count unbounded, drawing bounded by the visible rows); a text-entry widget; wrapped paragraphs past 6 lines with a line cap the caller sets |
| `icons` | `icon/{mod,path,raster,cache,sample}.rs`; `icon/image.rs` behind `png` | icon-theme lookup with `index.theme` inheritance (today hicolor-only, `modules/tray/theme.rs:13-19`); optional `svg` (below) |
| `wayland` | `outputs.rs`, `density.rs`, `region.rs`, `daemon/surfaces.rs`, `daemon/canvas.rs`, the Wayland half of the `poll` loop (`daemon/mod.rs`): moved by `scootui-extract-wayland`, with nothing new in that PR | in a **separate** PR (`scootui-surfaces-on-demand`): a layer-surface lifecycle that is *on demand* (created when there is something to show, destroyed when empty, no surface held at idle) beside the bar's one-per-output; a full-screen `xdg_toplevel` surface kind (scootgreet); the memfd name parameter in `scootbg-mem` |
| `keyboard` | none | keymap, modifiers, repeat, keysym to text ([below](#keyboard-and-text-entry)) |
| `control` | `control/{framing,paths,claim,conn}.rs` and the bounds (`MAX_CONNECTIONS`, `MAX_SUBSCRIBERS`, the write-stall deadline; `control/mod.rs:57-65`): moved by its own ticket, `scootui-extract-control` | a socket name that is a parameter instead of fixed to `scootbar-…` |

What stays in scootbar: modules, `bar.rs`, `layout.rs` (the three sections),
`render.rs` (the bar's per-module repaint, which is bar-shaped), config,
CLI, the popup's Wayland half (`daemon/popup/`, which is the bar's `xdg_popup` on its
own layer surface), the pointer bindings and actions.

`scootbg-mem` stays the one crate with `unsafe` code. `scootui-surfaces-on-demand` adds
the memfd name parameter that extract-scootui asks for (today every buffer is named
`scootbg-wallpaper`, `crates/scootbg-mem/src/shm.rs:173`), and scootgreet adds a locked,
zeroized page type there ([scootgreet.md](scootgreet.md#security-model)).

### `scoot-rt` (library, `crates/scoot-rt`): the leaf both depend on

The D-Bus client is not free of the bar's other modules. `dbus/conn.rs` and `dbus/link.rs`
call `crate::print::warn` (eight sites: `conn.rs:640,658,766,778,789,797,828,840`;
`link.rs:353,388`) and `crate::control::paths::runtime_dir` (`conn.rs:111,207`,
`link.rs:302`). If `print.rs` and `control/` moved into `scootui`, `scoot-dbus` would depend on
the drawing crate. **Decided: a tiny leaf crate, `scoot-rt`**, with std and `rustix` only,
holding:

- `print`/`warn` (scootbar's 30-line `print.rs`; scootbg carries a copy of the same three
  functions and can adopt it later);
- `runtime_dir` (`control/paths.rs:89-96`).

Both `scootui` and `scoot-dbus` depend on it, and neither depends on the other. The warnings'
hard-coded `scootbar: dbus:` prefix becomes the running binary's name, read once at startup.
The two alternatives were rejected:

- copying the ~45 lines into `scoot-dbus` is a third copy of `print.rs` to keep in step;
- passing a logger and the runtime dir into every `Conn` changes 11 call sites and the
  public API in a PR meant to be mechanical.

`scoot-rt` is created by `scootui-extract-draw`, and it gets its CI classify arm (fanning out
to every consumer) and CLAUDE.md scope entry in that PR.

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

It depends on `scoot-rt` (above) and on nothing that draws. What it needs that it lacks,
each added by the first consumer that needs it (not in the move PR):

- An error reply with a message body. `reply_error` sends a name only
  (`conn.rs:522`), and `org.freedesktop.Notifications.CloseNotification`
  answers an unknown id with "an empty D-Bus Error message", so that call is
  covered. For limits and bad arguments, a text body helps the client's user.
  (scootnotify)
- **A reply for an over-cap method call.** Today a message past `MAX_MESSAGE = 1 MiB`
  (`proto.rs:51`) is skipped. If it was a method call, the caller waits for its
  own timeout: libnotify's default is 25 s, during which `notify-send` hangs. The
  header fields (at most 64 KiB) are read before the body is discarded, but today they are
  matched only against pending replies. A small addition answers
  `org.freedesktop.DBus.Error.LimitsExceeded` to a method call's serial. (scootnotify;
  confirmed in the #579 review's read of the discard path.)
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
| **`kbvm`** (pure Rust XKB implementation; 0.2.0, MIT OR Apache-2.0, crates.io, checked 2026-10-12) | No C library. Keeps the family's "links only libc, libm, libgcc_s" property (scootbar's measurement, `lightest.md` M5/M6) | Young (0.2.0, about 22k downloads); dead keys and compose verified by the spike, but 918 KB stripped on the dev VM and two env vars to find anything on NixOS; one maintainer |
| **libxkbcommon** via the `xkbcommon` crate (0.9.0, MIT; already in `Cargo.lock` because scoot links it through Smithay) | The reference implementation, which every compositor and toolkit uses; compose tables; zero marginal closure on a scoot desktop | A C shared library in a binary family that has none; the spike proves the consumer stays `forbid(unsafe_code)` (decided: no wrapper crate) |
| Hand-rolled | Smallest | Wrong for any non-trivial layout; rejected |

**Spike result ([record](../spikes/scootui-keyboard.md)): `xkbcommon`, accepted by the maintainer on 2026-10-17 (Decisions needed 3).**
`kbvm` 0.2.0 passes the whole layout set including dead keys, but costs
918 KB of stripped binary on the dev VM (+630 KB `.text`) against this
plan's 300 KB bar, pulls 22 extra crates, and needs `XKB_CONFIG_ROOT`
(RMLVO) plus `XLOCALEDIR`/`XCOMPOSEFILE` (compose) set before it finds
anything on NixOS — where neither default path exists — failing silently
to an empty keymap when they are missing. `xkbcommon` 0.9.0 over
libxkbcommon 1.13.2 decodes every probe identically (27/27 across `us`,
`us(intl)`, `de`, `fr`, `dvorak`, two-group `us,ru`; the same strings
typed live through `scoot msg type` on all six sessions), costs 328 KB
stripped (+173 KB `.text`, the `.so` already resident in scoot), needs no
environment on NixOS, fails loudly, and is already in `Cargo.lock`
through Smithay. No wrapper crate: both spike bins carry
`#![forbid(unsafe_code)]` over the full client pattern including compose,
so the consumer is `unsafe`-free as written. Compose (the `XCompose` file)
is required for scootgreet only if a password can contain a composed
character. Both stacks compose it, and neither depends on the greeter's
locale (`C` works); what matters is finding the file, which libxkbcommon
does with zero configuration on NixOS. Secret fields take `key_get_utf32`
(no allocation, safe) into the caller's buffer; composed text comes back
as a `String` in 0.9.0, so a caller-buffer compose call around C
`xkb_state_key_get_utf8` via `State::get_raw_ptr` is a follow-up only if
the greeter's threat model demands composed-char passwords with no heap
copies.

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
  fixed allocation that is never resized, so no copy of the secret is left in freed heap.
  The caller supplies that buffer: scootgreet's lives in a locked, zeroized page from
  `scootbg-mem` ([scootgreet.md](scootgreet.md#security-model)), and scootlaunch's `--password`
  prompt uses a `zeroize`-on-drop buffer (`zeroize` 1.9, Apache-2.0 OR MIT).
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

Measured 2026-10-18 ([record](../spikes/scootbar/text-shaping.md)): NO-GO.
343.4 KiB of `.text` against the ~250 KiB gate; ~0.2 ms per 500-byte body,
inside the time budget, so size alone decides. v1 ships without shaping:
Arabic, Hebrew and Devanagari draw wrong; Latin, Cyrillic, CJK and
monochrome-fallback emoji are acceptable.

Line breaking for bodies: break at spaces and after CJK characters, with a hard break for
over-long words (what `popup/wrap.rs` does, minus its 6-line cap). The
`unicode-linebreak` crate is Apache-2.0, which is allowed by `deny.toml`, but at about
15 KB of tables it is not needed for v1.

Fallback fonts: raise `MAX_FALLBACKS` from 2 to 4 in `scootui` (CJK + emoji
+ symbols + the user's), as a per-consumer constant, so the bar's bound is
unchanged.

## Theme: the live-looks contract, Stylix and looks

**The theming contract for all three apps is the live-looks design**,
[`dev/research/live-looks-design.md`](../research/live-looks-design.md), merged in #578 and
#581 (`main` at `ac1cca18`). The apps adopt it; they do not define their own theming. Its §9 states what each app must do,
and this section restates that for the apps. Section numbers (§N) and decisions (DN) are
that document's.

### What the apps implement

1. **Read the look layer.** Each app reads its fragment at `<state>/look/<fragment>` (§4.1).
   `<state>` is `--state`, then `$SCOOT_LOOK_STATE`, then `$XDG_STATE_HOME/scoot`, then
   `~/.local/state/scoot`. `$SCOOT_LOOK_STATE` is exported only by a private (`--headless`)
   session, to its direct children (D10, §4.2). scootnotify's user unit therefore always
   reads the user's state.
   - The fragments are `notify.toml` and `launch.toml`, plus `greet.toml` for the greeter
     (§3.3, §9 item 6).
   - **Precedence at runtime is flags > the user's own file > the active look's fragment >
     the app's default**, merged on raw TOML keys before any derivation (§4.3).
   - An app never reads Stylix, a base16 file or the Nix look registry directly (§9 item 4).
2. **Take the tokens from §3.2's single list**, never a local palette.
   - Required: `background`, `foreground`, `dim`, `accent`, `urgent`.
   - Optional, with their defaults:

     | Token | Default |
     |---|---|
     | `surface` | `background` |
     | `hover` | `accent` |
     | `focus` | `accent` |
     | `border` | `dim` |
     | `accent-fg` | `background` |
     | `selection` | `accent` |
     | `selection-text` | `accent-fg` |
     | `match` | `hover` |

   - Fonts: `[fonts]` `ui`, `sans`, `mono`, `size`, `popup-size` (default `size`), `app-size`
     (default 12) and `terminal-size`. Pixels are `clamp(1, 256, (pt*4+1)/3)`.
   - Opacity: `[opacity]` `bar`, `popups`, `terminal` (default 1.0 each).
   - Shape: `[shape] radius` and the ring widths.
   - Per-app overrides: `[apps.<app>]`, keyed by the app's own dotted config key and
     allowlisted.
   - Anything more an app needs is requested in #578, never invented app-locally.
3. **Reload.**
   - The long-running app (scootnotify) implements `reload` on its control socket. It
     validates whole, keeps the last good, and answers applied or refused. It also takes
     `scope = "look"`: the new look is merged under a snapshot of the user file's last
     good raw table, so a swap never applies a half-edited user file (§4.4). `scoot look apply`
     drives it in its swap order (§4.5).
   - The per-launch app (scootlaunch) reads the link at every open.
   - The greeter is static ([below](#the-greeter-is-static)).
4. **Treat a look as untrusted data.** A look carries presentation only. Keys are
   allowlisted in one place, `scoot-look` (§3.4). Every value passes the contract's value
   grammar per field class: charset and length caps, no control characters or NUL (§3.5).
   Each app's loader enforces both again. scootnotify's `[[rule]]` and `exec`, and
   scootlaunch's `terminal`, are user config only and never allowlisted.
5. **Never wedge.** A dangling or missing link means "no look layer" plus a warning at
   startup, and a refused reload at runtime (§4.8).

### Stylix

Stylix reaches the apps **only as the synthesized `stylix` look**, through one importer
(`import-base16`, §5.3). There are no per-app `stylix.enable` options and no per-app slot reads.
The mapping (§5.3; the fix report's table):

| Token | Slot |
|---|---|
| `background`, `surface` | `base00` |
| `foreground` | `base05` |
| `dim`, `border` | `base03` |
| `accent`, `hover`, `focus` | `base0D` |
| `urgent` | `base08` |
| `accent-fg` | `base00` |
| `selection` | `base02` |
| `selection-text` | `base05` |
| `match` | `base0A` |

`sizes.popups`, `sizes.applications` and `opacity.popups` map to `popup-size`, `app-size` and
`opacity.popups`. Through it, scootnotify and scootlaunch draw what Stylix's own mako and
fuzzel targets draw. Those targets were checked at `fb28acd59e2ac1984ec84fa496599d6b4bf3e690`:
mako uses `base00` cards, `base0D` and `base03` borders, a `base08` critical border and a
`base02` progress bar; fuzzel uses `base02` selection with `base05` text, `base0A` match and a
`base0D` border. With a non-Stylix look, they draw what that look's mako and fuzzel draw today
(§9).

### The five requests, decided

All five requests this plan raised were accepted in #578 §9: `focus` in both fragments,
`fonts.popup-size` and `opacity.popups`, the fragment names and `[apps.<app>]`, a `greet.toml`
for the greeter, and `selection-text`.

### Per-app element tables

[scootnotify](scootnotify.md#stylix-and-look-integration),
[scootlaunch](scootlaunch.md#stylix-and-look-integration) and
[scootgreet](scootgreet.md#stylix-and-look-integration) map each UI element to a role. Each
mapping is chosen so the shipped looks draw what mako, fuzzel and ReGreet draw for them today:

- borders on `focus`, because `focus` differs from `accent` on moonrise and ginger-night (#578 B1);
- launcher selection on `selection`/`selection-text`;
- match on `match`;
- cards on `surface`.

Each table's "today" column is the evidence.

### Fonts and sizes

- **Family**: `fonts.ui` for scootnotify and scootlaunch. Fonts come from the look's carrier
  (D13), and the scootbar module's `fontFile` helper (`nix/modules/scootbar.nix:62-95`) becomes
  that carrier's Nix side (§9 item 3). scootgreet uses `fonts.sans`, the face ReGreet gets
  today (`nix/modules/nixos.nix:2456-2460`).
- **Size**: `fonts.popup-size` for scootnotify and scootlaunch (10 pt = 13 px by default);
  `fonts.app-size` for scootgreet (12 pt = 16 px, today's measured greeter size).

### The greeter is static

The login screen runs before any user session, as the `greeter` user, from `/etc` (§6.4). It
takes `programs.scoot.greeter.look`, which defaults to the declared look, rendered at system
build time. A live swap never touches it. A non-Nix `sudo scoot look apply --greeter` is
"later, not v1" in #578. greetd's `restartIfChanged = false` means a running greeter shows a
new look only from its next start ([scootgreet.md](scootgreet.md#stylix-and-look-integration)).

### Example configs

`docs/examples/<look>/` carries no colors for these apps (§9 item 5). Layout-only example
`notify.toml` and `launch.toml` (anchor, width, timeouts) may ship beside the bar's. A look's
colors reach the apps as rendered fragments, and per-app look values live in its
`[apps.<app>]` table.

## Live look swap

Covered by the contract ([above](#theme-the-live-looks-contract-stylix-and-looks)). What each
app adds:

- **scootnotify re-themes in place** on `msg reload` (and `scope = "look"`). Cards on screen,
  history, DND and ids survive, and the cost target is under 5 ms plus a font load.
  - Under Nix its unit never restarts for a config change, because a restart throws away what
    is on screen. Home Manager has `X-Reload-Triggers` (`modules/systemd.nix:140` at
    `efa3ccb4c3cc90d832eab232976379058fa75aa3`, the rev `scripts/scootbar-stylix-test.sh:35`
    pins), used with `ExecReload = scootnotify msg reload`.
  - On NixOS the unit is a *user* unit, and what `switch-to-configuration` does to it is **not
    verified here**. `notify-nix` checks it at the pinned nixpkgs (mind
    `nix/modules/nixos.nix:2398-2402`: nixpkgs warns when a unit sets both `reloadIfChanged`
    and `reloadTriggers`).
  - A look change is driven by the contract's own `scoot look sync` and `apply` (§4.5, §6.2).
- **No SIGHUP and no file watch**, for the contract's §4.4 reasons and scootbar's
  (`crates/scootbar/src/daemon/mod.rs:32-34`).
- **scootlaunch**: the next open reads the new link.
- **Optional hooks** (§7.3, §9 item 8):
  - scootnotify may show a "Look: NAME" card on scoot's `look` event;
  - scootlaunch's dmenu mode serves a `Look:` picker
    (`scoot look list | scootlaunch --dmenu | xargs scoot look apply`).

## Nix modules and the desktop profile switch

Each app follows the scootbar module layout (`nix/modules/scootbar.nix` plus
`scootbar-home.nix` plus `scootbar-nixos.nix`):

- `nix/modules/<app>.nix`: options (`enable`, `package`, `features`, free-form
  `settings` typed as TOML, `systemd.enable`) and the rendered `configFile`, which
  holds **only what the user wrote** (contract §5.2: no look or Stylix value is merged
  into the file; those arrive at runtime through the look layer). It reads the scoot desktop profile
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
  `--no-bar` is opt-out for the bar). A look reaches them through `scoot look apply`,
  like everything else (contract §6.5). It never installs scootgreet: replacing a login
  screen from a curl pipe is the kind of hard-to-reverse change CLAUDE.md asks
  explicit confirmation for. The greeter's install docs are a manual
  `/etc/greetd/config.toml` edit, with the VT-switch recovery path written next to it.

## Packaging and CI

- **CI path filters** (CLAUDE.md: "Every new package or component adds its path
  filter to the classify job … in the same PR that adds it"): each crate's first
  PR adds a classify output (`scootui`, `scoot_rt`, `scoot_dbus`, `scoot_xdg`, `scootnotify`,
  `scootlaunch`, `scootgreet`), arms for `crates/<crate>/Cargo.toml` (with
  `scoot=true version=true`, as scootbar's does, because the flake reads the
  description) and `crates/<crate>/*` before the catch-all, and a job gated on its
  output (`.github/workflows/ci.yml:194-355` holds the pattern). The shared crates fan
  out: a `crates/scootui/*` change sets `scootbar`, `scootnotify`, `scootlaunch`
  and `scootgreet`; a `crates/scoot-dbus/*` change sets every D-Bus consumer; a
  `crates/scoot-rt/*` change sets both of those sets; and a `crates/scoot-xdg/*` change sets
  scootlaunch and scootgreet. This is the rule CLAUDE.md states: "A change to shared code fans
  out to its dependents and nothing more".
- **Fuzz**: each app's targets join the fuzz job with fixed budgets (scootbar's
  are 1M to 5M runs, `ci.yml:1412-1450`).
- **Distro packages**: deb (`packaging/deb/build-deb.sh:93`'s package loop and the
  `deb-meta.py` tables), rpm (`packaging/rpm/<app>.spec`, with `%check` running
  `--check` on the example config), and Arch (`packaging/arch/<app>{,-git}/`).
  These come in each app's packaging ticket, after the binary is usable, not in its
  first PR.
- **Versioning**: every crate inherits the repository's one version
  (`version.workspace = true`; docs/versioning.md), and one `vX.Y.Z` tag
  releases every binary, so a new app needs no version policy of its own. Its
  Conventional Commits scope still matters: it groups the app's entries in the
  changelog (CLAUDE.md, Commits and CI). The new scopes `scootnotify`, `scootlaunch` and `scootui` are already named in CLAUDE.md.
  `scoot-rt`, `scoot-dbus`, `scoot-xdg` (the desktop-entry parser, split out of scootlaunch when
  scootgreet becomes its second consumer) and `scootgreet` are not: adding each to
  CLAUDE.md's scope list, with its CI classify arms, is part of the first PR that creates
  the crate.

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

Ordered by what unblocks what, and by risk. **Nothing is extracted until a gate says go.**

1. **Start now, all independent:**
   - (candidate) [`greet-headless-login`](../../docs/scootbar/backlog/greet-headless-login.md),
     the Wayland-free greetd login for agents and VM tests, on the go-ahead;
   - the two spikes, [`scootui-keyboard-spike`](../../docs/scootbar/backlog/resolved/scootui-keyboard-spike-done.md)
     and [`scootui-text-shaping-spike`](../../docs/scootbar/backlog/resolved/scootui-text-shaping-spike-done.md)
     (NO-GO, measured 2026-10-18),
     which decide the two choices that are expensive to reverse;
   - [`greet-baseline`](../../docs/scootbar/backlog/greet-baseline.md), measurement only;
   - the core ticket [`layer-shell-default-output`](../../docs/backlog/core/layer-shell-default-output.md),
     which fixes mako and fuzzel today.
2. **The gates, out of the workspace, on the go-ahead.** Each is a prototype that `#[path]`-includes scootbar's
   sources (`crates/scootbar/fuzz/Cargo.toml:28-65` already does this), so a no costs no
   refactor:
   - [`notify-prototype`](../../docs/scootbar/backlog/resolved/notify-prototype-done.md) can start at once;
   - [`launch-prototype`](../../docs/scootbar/backlog/resolved/launch-prototype-done.md) (GO, measured 2026-10-18) and
     [`greet-prototype`](../../docs/scootbar/backlog/greet-prototype.md) follow the keyboard
     spike.

   All three wait on the maintainer's go-ahead.
3. **The extraction, once any gate says go**, in mechanical PRs held to scootbar's ratchet:
   1. `scootui-extract-draw`, which also creates `scoot-rt`;
   2. then, in parallel: `scoot-dbus-extract`, `scootui-extract-wayland` and
      `scootui-extract-control`.

   Nothing new is added in these PRs. New code waits for
   `scootui-surfaces-on-demand`, `scootui-keyboard` and `scootui-entry-and-list`.
4. **scootnotify first.** It needs the D-Bus server role, pointer input, multi-line text and
   on-demand overlay surfaces, and no keymap. It has the clearest gain: two processes and a
   shell feed become one.
5. **scootlaunch second.** It is the first consumer of keyboard, entry and the virtualized
   list, and a typo in a launcher costs nothing.
6. **scootgreet last.** It has the same widgets, plus a security boundary and a login that
   must never be stranded. By then the entry widget has been in daily use in the launcher.

**The riskiest decisions**, and what retires each first:

- the keyboard stack (spike 1);
- shaping (spike 2);
- greetd's retained-session behavior (`greet-prototype`, against a real greetd);
- whether each app beats its incumbent on what can really fail (the three gates).

The scoot-hosted greeter is not on any path unless cage fails the GPU-free check.

## The tickets

### Where they live

All live in `docs/scootbar/backlog/` (area `scootbar`, milestone `M7`), the backlog that
already holds the three pointer entries this plan turns into epics. Two compositor-side
tickets, scoot's greeter host mode and the layer-shell default output, live in
`docs/backlog/core/`. `scripts/backlog` knows a fixed set of areas (`scripts/backlog`,
`BACKLOGS`). Adding `scootnotify`, `scootlaunch` and `scootgreet` areas is a tooling change
outside this docs-only PR.

### How the gates stay closed

`scripts/backlog` has only `open`, `research` and `resolved` (`STATUSES`, `scripts/backlog:59`).
`resolve` runs `unblock_dependents` (`scripts/backlog:628-650`), which strips a slug from every
dependent's `blocked` when it appears there as a list item. A gate ticket resolved as a
**no** would therefore unblock the build ticket it was meant to stop, and a `claim --next`
swarm would pick it up.

So **every ticket that a gate guards names the gate in prose**:
`blocked: "…, GO verdict recorded in notify-prototype (a human removes this clause)"`.
The regex matches a slug only at the start of the field or after `,`, `;`, `and` or `or`
(`scripts/backlog:634-635`). A slug preceded by "in " is not a list item, so it survives
`resolve`. This was checked by running the script's own regex on both forms.

When a gate is resolved:

- **go:** a human deletes that clause from the guarded tickets, which then unblock normally;
- **no:** a human resolves the epic and the guarded tickets as won't-do, with the gate's evidence.

No tooling change is needed. A `decision` status with a recorded verdict would make it
mechanical, and is in the [Deferred](#deferred) list.

The guarded tickets are:

- `notify-server`, guarded by `notify-prototype`;
- `launch-desktop-index`, guarded by `launch-prototype`;
- `greet-greetd-client`, guarded by `greet-prototype`;
- `scootui-extract-draw`, which needs a go from *any one* of the three gates.

### Roots and edges

**The roots wait on the maintainer's go-ahead**, except the four items in Start order 1,
which are unblocked. Everything else names its predecessors by slug. The apps' tickets
that read look fragments are blocked on #578's `look-format` (the `scoot-look` crate).
That slug exists on `main` since #578 merged.

Shared:

| Slug | What | Serves | Blocked by |
|---|---|---|---|
| `extract-scootui` (epic, existing) | the shared foundation | both | its children |
| `scootui-keyboard-spike` | kbvm vs xkbcommon, measured | both | none: start now |
| `scootui-text-shaping-spike` | rustybuzz + bidi cost, measured | daily-drive | none: start now |
| `scootui-extract-draw` | move pure drawing, widgets, icons into `crates/scootui`; create `scoot-rt` | both | a GO from any gate (prose) |
| `scoot-dbus-extract` | move the D-Bus client into `crates/scoot-dbus` | both | `scootui-extract-draw` |
| `scootui-extract-wayland` | move outputs, layer surfaces, shm, the poll loop's Wayland half; nothing new | both | `scootui-extract-draw` |
| `scootui-extract-control` | move `control/{framing,paths,claim,conn}.rs`; the socket name a parameter | both | `scootui-extract-draw` |
| `scootui-surfaces-on-demand` | the on-demand layer lifecycle, the `xdg_toplevel` kind, the memfd name | both | `scootui-extract-wayland` |
| `scootui-keyboard` | keymap, modifiers, repeat, keysym to a caller's buffer | both | `scootui-keyboard-spike`, `scootui-extract-wayland` |
| `scootui-entry-and-list` | text entry (password mode), virtualized list, focus chain | both | `scootui-keyboard` |

The per-app tables are in each app's doc:
[scootnotify](scootnotify.md#tickets), [scootlaunch](scootlaunch.md#tickets) and
[scootgreet](scootgreet.md#tickets).

## Decisions needed

Only the items that are the maintainer's call are asked. The rest are recorded as
**defaults**: decided here with evidence, and changeable by anyone with a reason.

**The maintainer's call:**

1. **Go-ahead and queue.** The gated work is blocked on it.
   **Given 2026-10-17 for `scootlaunch` and `scootnotify`** (the maintainer asked for both, launcher
   first); the `scootgreet` tickets stay gated.
   *Recommend:*
   - start now: the two spikes, `greet-baseline`, and `layer-shell-default-output`. The last is
     unblocked in this PR because it helps mako and fuzzel today;
   - start the three prototypes on the go-ahead: `notify-prototype` at once, and the
     launcher's and greeter's after the keyboard spike;
   - hold `scootui-extract-draw` until a gate says go.
2. **The greeter's name.** *Recommend:* `scootgreet` ([Names](#names)).
3. **A C library in the shell family**: `libxkbcommon`. **Decided 2026-10-17: accepted.** The
   maintainer confirmed it is linked dynamically (`libxkbcommon.so.0`), not compiled in. The
   spike chose it because `kbvm` passes the layout set but costs 918 KB of stripped binary
   against the 300 KB bar, and needs `XKB_CONFIG_ROOT`, `XLOCALEDIR` and `XCOMPOSEFILE` set or
   it fails silently on NixOS. No wrapper crate is needed: the consumer keeps
   `forbid(unsafe_code)`. The scoot compositor already links the library, so a scoot desktop
   gains nothing; the shell apps lose their libc-only property.
4. **The `resvg` size waiver** for SVG app icons in scootlaunch. *Recommend:* default-on `svg`,
   PNG preferred when the theme has one, waiver ruled on the measured row (as the tray's were).
5. **The greeter host.** *Recommend:* cage. A scoot kiosk host only if cage fails the GPU-free
   check in `greet-baseline`. Whether to add that compositor attack surface at the login
   screen at all is the maintainer's call.
6. **(a) Which output an output-less layer surface goes to**, the one wlr-layer-shell calls
   "most recently interacted with". *Recommend:* the output under the pointer, as scoot's new
   windows already do (`protocols.md:1017-1021`). That is in `layer-shell-default-output`.
   The alternative is the keyboard-focused window's output; it gives nothing on an empty
   workspace, and IPC has no focused-output field. This changes mako and fuzzel today.
7. **(b) App scopes in scoot's own spawn path.** Binds spawn into `scoot.service`'s cgroup
   (`crates/scoot/src/compositor/state.rs:1733-1803`, a plain `Command`). *Recommend:* yes,
   as a later core ticket filed once `launch-spawn` proves the scope code, so Super+Return
   apps get `app-scoot-…scope` too. It is not filed in this PR.
8. **(c) The dmenu mode refuses unknown fuzzel flags loudly** (exit 2, listing the supported
   ones). *Recommend:* yes. A flag that takes a value, silently ignored, eats the next argument.
   The repo's own call sites are pinned to the supported set, and the opt-in period is when a
   user's personal scripts surface. The alternative is warn-and-ignore for flags that take no
   value.
9. **(d) scootlock or scootgreet first** (they share the widget set). *Recommend:* decide on
   `greet-baseline`'s number, not now.
   - The most scootgreet can remove is what ReGreet adds beyond cage: 638.4 MiB (166 paths),
     because cage stays the host (cage alone is 380.2 MiB; #579 re-review, pinned nixpkgs). The
     real saving is that figure minus the GTK and GStreamer libraries the installed desktop
     profile already carries, and it may be much smaller.
   - scootlock's incumbent (swaylock) is already small, but a lock screen is used many times a
     day.
   - The rule: **scootgreet first** if its marginal saving over the installed profile is at least
     about 300 MiB, or if the scoot-iso image is still over its size cap. That is the
     `greet-prototype` go condition. **Otherwise scootlock first**, or neither.
   - The computer-use half of the greeter, a headless login, does not wait on either: it is
     [`greet-headless-login`](../../docs/scootbar/backlog/greet-headless-login.md), a candidate
     early ticket.
10. **(e) The greeter fallback: agreety or ReGreet.** *Recommend:* agreety. It is text-only,
    ships in greetd's own package, and needs no compositor, GPU or font: exactly what the
    fallback exists to survive.
11. **(f) The greeter's default flip and screen-reader users.** *Recommend:* the flip keeps
    ReGreet the default whenever the system enables a screen reader, and the installer offers
    the choice ([scootgreet.md](scootgreet.md#decisions-specific-to-the-greeter)).

**Defaults (decided here, not the maintainer's call):**

- **No backlog area per app yet.** It is a `scripts/backlog` change (three
  `BACKLOGS`/`RESOLVED`/`INDEXES` rows plus a test) and a `git mv` of open entries, made when
  each app's first code lands.
- **`scoot-dbus` is separate from `scootui`, with `scoot-rt` as their shared leaf**
  ([above](#scoot-rt-library-cratesscoot-rt-the-leaf-both-depend-on)).
- **Shaping in scootnotify's default build** follows the spike's gate. It escalates to the
  maintainer only if the gate fails.
- **No notification history on disk** (privacy). An opt-in `history-file` can come later.
- **Theming through #578.** This is the contract owner's decision (#578 D5, D9, D14). This plan
  follows it.

## Deferred

These were found by the #579 review and are deliberately not done here, each with its reason:

- **A `decision` ticket status in `scripts/backlog`.** The prose-clause mechanism above covers
  the hazard with no tooling change. A status would need its own PR with tests.
- **The bar's `scootbg` copy of `print.rs`** stays until scootbg is moved onto the shared crates,
  which is a later ticket behind scootbg's release gate.
- **A message catalog for UI strings** (greeter and notifications). v1 is English, stated as a
  regression risk in [scootgreet.md](scootgreet.md#what-the-screen-offers). Whether ReGreet
  localizes is unverified.
- **RTL caret and bidi in the launcher's entry**: v1 states it. IME is a follow-up (fuzzel has
  none, so there is no regression).
- **Greeter screen blanking:** cage has no output-power support. Stated, not solved.
- **`greet-scoot-kiosk-spike` and `scoot-greeter-mode`** stay filed but are blocked on cage
  failing the GPU-free check, so nothing starts them otherwise.

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
- **A per-app Stylix mapping and `programs.<app>.stylix.enable`** (this plan's first
  draft, modeled on scootbar's). The contract shows why it cannot coexist with live
  swapping: a Stylix value baked into the app's file sits above the look layer, so a
  swap could never change it (#578 §5.1).
- **Moving scootbg onto `scootui` now.** Its release gate makes that its own
  ticket, later.
