# scootgreet: design

Design pass, 2026-10-12. This covers the login greeter: what it does, how it is
built, and how it can never lock a user out. The parts it shares with scootnotify and
scootlaunch are in [shell-apps.md](shell-apps.md). The tickets are under
[the epic](../../docs/scootbar/backlog/scootgreet.md). The name is a
recommendation ([shell-apps.md, Names](shell-apps.md#names)).

Sources:

- greetd's own man pages and source (`kennylevinsen/greetd`, master, fetched
  2026-10-12): `greetd-ipc(7)`, `greetd(1)`, `greetd(5)`, `greetd/src/{main,context,server}.rs`,
  `greetd_ipc/src/lib.rs`.
- The pinned nixpkgs (`8ce4ef6cb6f871616146b9fe26d2a5ae594e94fe`):
  `nixos/modules/services/display-managers/{greetd,regreet}.nix`.
- ReGreet 0.5.0's source.
- `org.freedesktop.login1(5)`.

## Contents

1. [Goals, non-goals and the case for it](#goals-non-goals-and-the-case-for-it)
2. [Today: ReGreet under greetd and cage](#today-regreet-under-greetd-and-cage)
3. [The greetd client](#the-greetd-client)
4. [What the screen offers](#what-the-screen-offers)
5. [Which compositor hosts it](#which-compositor-hosts-it)
6. [Security model](#security-model)
7. [Failure modes: never strand a login](#failure-modes-never-strand-a-login)
8. [Architecture and what the shared crate must offer](#architecture-and-what-the-shared-crate-must-offer)
9. [Stylix and look integration](#stylix-and-look-integration)
10. [The installer ISO](#the-installer-iso)
11. [Testing and verification](#testing-and-verification)
12. [Docs and packaging](#docs-and-packaging)
13. [Decisions specific to the greeter](#decisions-specific-to-the-greeter)
14. [Tickets](#tickets)

## Goals, non-goals and the case for it

**Goals.** A greetd greeter that draws the session's own look with the same text
and tokens. It logs in through any PAM conversation (password, OTP,
fingerprint) and offers power actions through logind. It runs GPU-free, and it never
leaves the machine without a way to log in.

**Non-goals (v1)**, decided:

- X11 sessions (`xsessions/`). scoot is Wayland, and starting an X session from greetd
  needs a wrapper per distro. A `x-session-wrapper` key can add them later.
- Typing a free-form session command. ReGreet allows it. It is not a security hole, since it
  runs after authentication as the user, but it is a power-user feature that a config file
  covers.
- **Screen readers.** ReGreet is a GTK4 app and speaks AT-SPI. scootgreet does not, and
  nothing in the shared stack does. This repo has valued that before: the
  greeter-leak fix rejected `NO_AT_BRIDGE=1` because it "blinds screen readers at the login
  screen" (`docs/backlog/resolved/greeter-session-leak-done.md:71-75`). So **ReGreet
  stays a supported option indefinitely**, the docs say which greeter to pick for a
  screen reader, and this is [Decision 3](#decisions-specific-to-the-greeter).
- An on-screen keyboard, user creation, and autologin. Autologin is greetd's
  `initial_session`, and this repo never sets it on an installed system
  (`nix/modules/nixos.nix:27-36`).

**Why build it instead of keeping ReGreet**:

- *Weight.* ReGreet 0.5.0 pulls in GTK4, GStreamer (base and good), pango,
  librsvg, AccountsService and glib (nixpkgs' `regreet` build inputs), runs under
  `dbus-run-session`, and needs `accounts-daemon` enabled
  (nixpkgs `regreet.nix` L158-159). **Nothing in this repo has measured its
  closure, RSS or time to first frame**, so the first ticket
  (`greet-baseline`) does, as scootbar's M0 did. scootgreet needs no session bus, no
  GTK and no media stack. That alone removes the class of leak
  `greeter-session-leak-done.md` fixed (`dbus-run-session` and ReGreet's AT-SPI
  `dbus-daemon` surviving every greeter session: 85 closing sessions and 137 greeter
  processes measured on the M2).
- *Look consistency without a CSS minefield.* Each look ships a hand-tuned
  `regreet.css`, and a CI gate exists only because a bare `frame {}` rule also
  inks ReGreet's empty notification bar, leaving a blank pill above Reboot/Power Off
  (`scripts/check-regreet-css-scope.sh`; `site/src/content/docs/scoot/theming.md:108-125`;
  verified against ReGreet 0.5.0's `src/gui/templates.rs`, where the third frame
  carries no `background` class). A greeter that draws its own widgets from tokens
  cannot have that bug. Today's look also does not reach ReGreet's cursor or GTK theme:
  `theme-look.nix:61-69` says the greeter gets the look's cursor, but nothing sets
  `services.displayManager.regreet.cursorTheme` or `.theme`, so the login screen shows
  Adwaita's.
- *Testable login.* ReGreet 0.5 shows the user and session with a Login button but **no
  password field until Login is clicked**. The `scoot-iso` QEMU test clicks fractional
  coordinates to reveal it (`scoot-sh/scoot-iso` at `75994a8`, `scripts/qemu-test.sh:667-668,719`:
  "ReGreet 0.5 shows User+Session preselected with a Login button but no password field:
  clicking Login reveals it"). ReGreet also starts the session only after it exits, so the M2's
  scripted login has to kill `dbus-run-session` by hand. scootgreet focuses the password
  field at once, exits the moment `start_session` succeeds, and offers a headless
  `scootgreet login` for VM test harnesses ([below](#the-greetd-client)). That serves
  computer use: an agent bringing up a VM logs in with one command or one typed line.

**What we lose**: AT-SPI (above), GTK's input stack, ReGreet's maturity and
features (environment variables per session, demo mode), and its CSS theming for
users who wrote their own.

**Go/no-go gate** (at `greet-ui`): closure and RSS at most a third of the measured
ReGreet + cage + `dbus-run-session` stack. The baseline sets the real numbers.
First frame no later than ReGreet's. A full PAM matrix on the dev VM passes:
password, wrong password, an expired-password change, an OTP second prompt
(`pam_oath` or a test module), and an info-only message. Otherwise keep ReGreet.

## Today: ReGreet under greetd and cage

From `nix/modules/nixos.nix` and the pinned nixpkgs' `greetd.nix` and `regreet.nix`:

- `programs.scoot.greeter.enable` (`nixos.nix:989-1030`, aliased as
  `programs.scoot.desktop.greeter` at `:167-170`) turns on nixpkgs'
  `services.displayManager.regreet`. The greetd command is
  `dbus-run-session cage -s -d -m last -- regreet` (cage args set at `nixos.nix:2352-2357`:
  `-m last` because cage otherwise spans every output, and a 2560×1600 plus 1920×1080 pair
  drew a 4480×1600 window).
- greetd: `vt = 1` fixed by nixpkgs (`services.greetd.vt` was removed with "The VT is now
  fixed to VT1."), `default_session.user = "greeter"`, `Conflicts=getty@tty1`,
  `autovt@tty1` disabled, `Restart = "on-success"` when there is no `initial_session`,
  and `restartIfChanged = false`.
- The look block (`nixos.nix:2421-2475`, gated on `theme.targets.greeter.enable`,
  `desktop.nix:1452-1460`) sets the backdrop to the look's wallpaper, the GTK dark
  setting, the look's `regreet.css`, and the font (`fonts.sans`, 12 pt). A Stylix
  regreet target and a set `greeter.background` are refused together (`nixos.nix:86-88`).
- `KillUserProcesses` scoped to `KillOnlyUsers = [ "greeter" ]` (`nixos.nix:2381-2382`)
  stops each greeter session's leftovers.
- The design comment says the greeter stays outside scoot: "hosting a pre-login greeter in
  the compositor would need a locked-down scoot profile (no binds, no IPC socket), which
  is out of scope" (`nixos.nix:2335-2338`). This plan keeps cage as the first host and
  makes the scoot host a separate, opt-in phase.

## The greetd client

**Wire** (`greetd-ipc(7)`): the socket path comes from `GREETD_SOCK`. Each message is
"a 32-bit integer in native byte order" followed by "a UTF-8-encoded JSON string" of that length.

| Request | Fields |
|---|---|
| `create_session` | `username` |
| `post_auth_message_response` | `response` (optional: "If the message was informative (info, error), then a response does not need to be set") |
| `start_session` | `cmd` (array), `env` (array; `#[serde(default)]` in `greetd_ipc`) |
| `cancel_session` | — |

| Response | Fields |
|---|---|
| `success` | — |
| `error` | `error_type` (`auth_error`, "not a fatal error, and is likely caused by incorrect credentials", or `error`), `description` |
| `auth_message` | `auth_message_type` (`visible`, `secret`, `info`, `error`), `auth_message`. "There are no limits on the number and type of messages … a greeter should not make any assumptions about the messages. Must be answered with either post_auth_message_response or cancel_session." |

The field names are `auth_message_type` and `auth_message`. The `greetd_ipc` crate's
doc example uses stale names (`message`, `message_type`); the man page and the
server's types use the real ones. scootgreet does **not** depend on `greetd_ipc` (it pulls in
serde and a tokio-optional codec): the frame is four bytes plus JSON, and the
password frame must be built by hand anyway ([Security](#security-model)).

**Conversation state machine** (pure, in `src/greetd/conversation.rs`):

```
Idle --(user chosen + Enter)--> create_session{username}
  auth_message visible  -> show the prompt text over a visible entry; Enter -> post_auth_message_response{response}
  auth_message secret   -> show the prompt text over a masked entry; Enter -> post_auth_message_response{response}
  auth_message info     -> show the text (dim); post_auth_message_response{} at once
  auth_message error    -> show the text (urgent); post_auth_message_response{} at once
  success               -> start_session{cmd, env} -> success -> exit 0
  error auth_error      -> "Login incorrect" (or its description); session is cancelled by greetd ("the session is cancelled automatically on error"); clear the entry, back to the password step via a fresh create_session
  error error           -> show the description; back to Idle
Escape at any step      -> cancel_session; back to Idle
```

- Messages are never assumed in number or order, so OTP, password-change ("current, new,
  retype") and fingerprint flows work. `pam_fprintd` sends an info message and then blocks
  the PAM stack until a finger. The greeter shows the text, answers the info, and waits, with
  Escape cancelling.
- While a request is in flight (PAM's delay after a wrong password is often 2 s), input is
  disabled and a "Checking…" line is shown. The loop stays responsive: the greetd socket is
  non-blocking in the `poll` set.
- **Invariant: scootgreet exits 0 only after `start_session` succeeded.** Every other exit
  (a fatal error, a lost compositor) is non-zero, so the wrapper's fallback catches it. A clean
  exit without a session would send greetd round its restart loop, which the fallback never
  sees ([Failure modes](#failure-modes-never-strand-a-login)).
- After `start_session` succeeds, **exit at once**: "The session will start after the
  greeter process terminates". greetd gives the greeter 5 s, then SIGTERMs every second, then
  SIGKILLs at 10 s (`greetd/src/context.rs:294-314`).
- A dropped connection cancels the session on greetd's side, and the greeter reconnects.
- Bounds: frames at most 64 KiB (refuse larger and reconnect), messages kept on screen at most 4 (the
  newest), each at most 512 bytes and stripped of control characters.

**`start_session`.** `cmd` is the chosen session's `Exec`, split by the Desktop Entry
quoting rules (the parser comes from scootlaunch's index; see [Architecture](#architecture-and-what-the-shared-crate-must-offer)).
`env` sets `XDG_SESSION_TYPE=wayland`, `XDG_SESSION_DESKTOP` (the first of the entry's
`DesktopNames`, else the file id) and `XDG_CURRENT_DESKTOP` (`DesktopNames` joined by
`:`). The scoot session launcher already handles a login that "arrives with `wayland`
already" (`resources/scoot-session:474-475`).

**`scootgreet login` (headless).** For VM tests and agents: `scootgreet login --user U
[--session NAME]`. It runs the same state machine against `$GREETD_SOCK`, reads each
`secret` answer as a line from stdin, prints `visible`, `info` and `error` messages to
stderr, refuses a `visible` prompt unless `--answer TEXT` covers it, and exits 0 once the
session is started. It has to run as the greeter user with the greeter's socket, exactly
as the M2's existing scripted login does. No new privilege is introduced.

## What the screen offers

**Layout.** A full-screen `xdg_toplevel` asks for fullscreen and draws the background (the look's
wallpaper, else the `background` token) with a centered card holding:

- a clock;
- the user (a list when several; the last user preselected; "Other…" to type a name);
- the PAM prompt and its entry, focused at once;
- the session selector (the last session for that user preselected);
- a message line;
- power buttons (Suspend, Reboot, Power Off) with a confirm step, as scootbar's power menu has.

An `xdg_toplevel` is used rather than a layer surface: it runs under cage (whose layer-shell
support was not verified here), under scoot and under sway alike.

**Keyboard-only flow.** Tab and Shift+Tab move through user, entry, session and power. Up and Down
change the user or the session. Enter submits. Escape cancels a conversation. The pointer works
too. The focus ring is the `accent` token, 2 device pixels.

**Users.** Read `/etc/passwd` directly (no NSS, no `libc` FFI). Keep UIDs in
`UID_MIN`..`UID_MAX` from `/etc/login.defs` (defaults 1000 and 60000, the range tuigreet
uses), drop shells ending in `nologin` or `false`, and apply config `hide-users` /
`show-users`. **NSS-only users** (LDAP, sssd, systemd-homed) are not listed. They use "Other…"
and type their name, which always works, and the docs say so. Avatars come from
`/var/lib/AccountsService/icons/<user>` when the file is readable (AccountsService's
icon directory: its `meson.build` `ICONDIR` and `daemon.c` `icon_filename`). Only the
file is read; there is no D-Bus call. Otherwise an initial tile. The rejected alternative is
AccountsService over D-Bus, ReGreet's way: it makes a system daemon a login dependency
for a list that `/etc/passwd` gives directly.

**Sessions.** `$XDG_DATA_DIRS/wayland-sessions/*.desktop` (nixpkgs puts the session
packages' `share` on `XDG_DATA_DIRS` for the display manager), with `Name` localized,
`Exec`, `TryExec`, `Hidden`, `NoDisplay` and `DesktopNames`, under the same id and
precedence rules as the launcher. The default is the user's last session, else config
`default-session` (the profile sets `"scoot"`), else the first.

**Remembered state.** `/var/lib/scootgreet/state` (owned by `greeter`, created by
tmpfiles as ReGreet's `/var/lib/regreet` is) holds the last user and the last session per user,
at most 16 KiB, written atomically after a successful `start_session`. Usernames and
session ids only, nothing secret.

**Power** through logind on the system bus (`scoot-dbus`): `CanPowerOff`,
`CanReboot` and `CanSuspend` decide which buttons show. `yes` and `challenge` show; `na`,
`no` and `inhibited` hide, with a tooltip for the reason. The calls are `PowerOff(false)`,
`Reboot(false)` and `Suspend(false)`. `interactive = false`: there is no polkit agent at the
login screen, and the man page names these calls "the primary mechanism to
poweroff/reboot/suspend/hibernate". Config can override any action with an argv (ReGreet's
`[commands]` equivalent) for systems without logind.

**Keyboard layout.** The host compositor owns the keymap (cage and scoot both take
`XKB_DEFAULT_*`). The greeter reads the layout names from the keymap and the active group
from `wl_keyboard.modifiers`, and shows "English (US)" beside the entry. When more than one
group exists, it shows the switch hint from the keymap's options (for example "Alt+Shift"). A
client cannot switch the compositor's group, and scoot has no layout-switch action
either (`site/src/content/docs/msg/requests.md:147-152`). The NixOS module passes
`services.xserver.xkb.{layout,variant,options,model}` into the greeter command's
environment, so the login keymap is the user's ([greet-nixos](../../docs/scootbar/backlog/greet-nixos.md)).
**Caps Lock on** shows a warning under the password entry in the `urgent` token. Num Lock
shows as a small indicator.

**Accessibility basics** (beyond keyboard-only): a `text-scale` option (1.0 to 2.0), a
`high-contrast` option that forces black and white tokens, no animation, and every action
reachable without a pointer. No screen reader (above).

**Outputs.** The card goes on one output: config `output` (a name), else the output with the
pointer, else the first. Every other output gets the background. Hotplug moves the card if its
output leaves. Fractional scale comes through `wp-fractional-scale-v1` + `wp-viewporter`, as the bar. Under
cage with `-m last` there is one output. Under scoot every output is used.

## Which compositor hosts it

| Host | For | Against |
|---|---|---|
| **cage** (today's host for ReGreet; phase 1) | Proven here; exits when its app exits (cage's `sigchld_handler` calls `server_terminate`); tiny; already in the NixOS module | A second compositor (wlroots) in the closure; its software-rendering fallback on a GPU-less box is **not verified here** (`greet-baseline` checks it on the dev VM, CLAUDE.md's GPU-free rule) |
| **scoot in a kiosk mode** (phase 2, opt-in, after a spike) | One compositor in the closure; pixman by default, so GPU-free by construction; the same cursor and output handling as the session; a logind-to-logind DRM handoff into the user's scoot | Real core work and a new security surface in the compositor ([below](#what-scoot-would-need)); a scoot bug then also takes out the login screen |

**Phase 1 command**: `cage -s -d -m last -- scootgreet`. There is no `dbus-run-session`:
scootgreet uses only the system bus, which removes the leak class without the
`KillOnlyUsers` workaround (the workaround stays for ReGreet users).

### What scoot would need

Spike `greet-scoot-kiosk-spike` (research), then core ticket `scoot-greeter-mode`.
Everything here is verified absent today:

- **No IPC socket.** Today `ipc::init` runs unconditionally and is fatal on error
  (`crates/scoot/src/compositor/mod.rs:449`). The socket is same-uid only
  (`listener.rs:240-307`), but a greeter-user socket that can `spawn` is a shell at the login
  screen if anything else ever runs as `greeter`.
- **No default binds except VT switching.** Today 45 binds are built in, including a terminal
  (`Super+Return`) and quit (`config.rs:4419` test), and there is no no-binds mode. Binds can only
  be removed one at a time (`config.rs:1750-1762`). `Ctrl+Alt+F1..F12` stay ("always enforced
  and cannot be unbound", `config.rs:1683-1688`), which is what keeps a VT escape at the login
  screen.
- **No user config**: config from an explicit root-owned path only, never `XDG_CONFIG_HOME`.
  Also no `[autostart]`, and no XWayland (it is off by default and has no `--no-xwayland` flag,
  since flags can only say yes, `cli.rs:697-710`, so the kiosk mode must refuse it outright).
- **Exit with the child**, returning its status. Today scoot "does not exit when it exits"
  (`site/src/content/docs/scoot/index.md:104-108`).
- **A reduced global list**: no data-control, virtual input, screencopy or foreign-toplevel
  management.
- **VT and DRM handoff.** On tty1 the greeter's compositor holds DRM master through logind.
  It must exit, and so release master, before greetd starts the user session.
  `vt-switch-back-greeter-master-done.md` shows what a greeter holding master costs a
  seatd-backed scoot on another VT. The seatd race is avoided by logind's design, and
  the logind path is still unmeasured (`docs/backlog/testing/logind-session-reconnect-test.md`).
  The spike measures a scoot-kiosk greeter handing over to a scoot user session on the M2.

Phase 2 is opt-in (`programs.scoot.greeter.host = "scoot"`), and cage stays the default
until the spike and the core ticket are reviewed and have run on the M2 for two weeks.

## Security model

- Runs as the `greeter` system user (greetd's default). It reads only
  `/etc/scoot/greet.toml`, `/etc/passwd`, `/etc/login.defs`, the session directories,
  AccountsService icon files, its state file, fonts and the wallpaper. It writes only its state
  file. There is no session bus and no network.
- **The secret's path is as short and as cleared as the language allows:**
  - Keystrokes go into the entry's fixed-capacity buffer, zeroized on clear and on
    drop, never resized ([shell-apps.md](shell-apps.md#keyboard-and-text-entry)).
  - Enter builds the `post_auth_message_response` frame by hand into a second
    fixed, zeroized buffer: JSON-escaping `"`, `\` and control characters inline. **Not**
    `serde_json::to_string`, which would leave a non-zeroized `String` behind.
  - The frame is written to the socket, and both buffers are zeroized before the next loop turn.
  - No `Debug` or `Display` implementation exposes entry contents, logging records message
    *types* and never responses, and `visible` answers (a username, an OTP) are not logged either.
  - Process dumps are disabled at start (`PR_SET_DUMPABLE` 0). Whether `rustix` exposes it and
    `mlock` safely under `forbid(unsafe_code)` is **to verify in `greet-greetd-client`**. If
    `mlock` is not available safely, the residual risk (the page reaching swap) is documented.
    greetd itself `mlockall`s (`greetd/src/main.rs:44`).
- Lockout and rate limits are PAM's job (`pam_faillock`). The greeter only disables input while a
  request is in flight.
- Untrusted text: GECOS names, session names and PAM messages are bounded and stripped of control
  characters. No markup is interpreted anywhere.
- **Nothing runs commands**: no terminal, no run box, no free-form session command, and the
  power overrides are argv from root-owned config only.
- No IME on the password entry (an IME candidate popup is the one non-lock element scoot draws
  over a lock screen, `protocols.md` text-input, and it would show the text).

## Failure modes: never strand a login

CLAUDE.md treats stranding a user out of their own desktop as the harm to design against.
Checked against greetd's source and the pinned nixpkgs:

- **When the greeter exits without a session**, greetd's `check_children` returns "greeter
  exited without creating a session", and `main` prints it and returns normally
  (`greetd/src/main.rs:44-56`: the error is printed with no non-zero `exit`). So greetd exits
  0, and nixpkgs' `Restart = "on-success"` restarts it with a fresh greeter.
- **A crash loop still strands the login.** The pinned `greetd.nix` sets no start limit, so
  systemd's default applies (5 starts in 10 s). A greeter that dies at once on every start
  (a bad font, a bad config) hits it, greetd stays failed, `getty@tty1` is in conflict, and
  VT1 is dead.

So:

1. **A bad look or config never stops the greeter.** A config that fails to parse is replaced by
   built-in defaults (tokens compiled in, the plain default theme), with an error line on screen
   ("config error, using defaults") and in the journal. A wallpaper that fails to load gets the
   `background` fill. A missing font falls down the chain to the well-known fonts (the bar's
   list, `crates/scootbar/src/font.rs:52-67`).
2. **The greetd command is a wrapper, `scootgreet-session`**, which runs the host
   (`cage … -- scootgreet`) and, **if it exits non-zero, execs `agreety --cmd <default
   session>`**. agreety is greetd's own text greeter, already used as `scoot-iso`'s live
   `default_session`. A greeter that cannot draw at all (no font anywhere, no outputs, a
   crash) degrades to a text login on the same VT, never to a dead VT. **To verify in
   `greet-baseline`:** that cage's exit status reflects the child's (otherwise scootgreet writes a
   one-byte "session started" marker under `$XDG_RUNTIME_DIR` before exiting 0, and the wrapper
   keys on that).
3. **VT escape.** `Ctrl+Alt+F2` reaches a getty on tty2: nixpkgs disables only `autovt@tty1`.
   `greet-nixos` verifies this on the dev VM, and the install docs print it next to the switch.
4. **A greetd socket error or protocol error** shows "Login service unavailable" with the
   power buttons, and retries the connection every 2 s.
5. **`start_session` failing** (a broken session `Exec`) shows the description and returns to
   Idle. Another session can be chosen.

## Architecture and what the shared crate must offer

`crates/scootgreet`, a binary, `#![forbid(unsafe_code)]`, single thread.

```
src/main.rs, cli.rs, config.rs (+tests; built-in defaults on any error)
src/greetd/   frame.rs (+fuzz), conversation.rs (pure), secret.rs (the zeroized frame)
src/users.rs  /etc/passwd + login.defs (+fuzz), avatars
src/sessions.rs  wayland-sessions via the shared desktop-entry parser
src/power.rs  logind through scoot-dbus
src/ui/       card layout, focus chain, widgets from scootui, wallpaper
src/state.rs  the remembered user and session
```

**Dependencies.**

- `scootui`: `widgets`, `icons` + `png`, `wayland` (with an `xdg_toplevel` fullscreen surface
  beside the layer surfaces), `keyboard`.
- `scoot-dbus`, `scootbg-mem`, the wayland crates, `rustix`, `serde_json` (responses only),
  `toml`, `zeroize`.
- `zune-jpeg` for JPEG wallpapers: Stylix's `image` is often a JPEG, and scootbg already
  uses this crate (`crates/scootbg/Cargo.toml`).

The desktop entry parser moves out of scootlaunch into a small shared `scoot-xdg` crate
when scootgreet becomes its second consumer. That is the same two-consumer rule as
`scootui`, and it lands in `greet-ui`.

**What the shared crate must offer.** This app is where the input-heavy widget set first
matters, more than the launcher:

- the text entry with a **password mode** (masked, zeroized, no copy, no IME), a byte cap,
  and caret editing ([shell-apps.md](shell-apps.md#keyboard-and-text-entry));
- **a focus chain** across several focusable widgets (Tab and Shift+Tab), with a visible focus
  ring. The bar's popup has a single list;
- **an inline selector** (user, session) drawn in the same surface, with no `xdg_popup` needed;
- **key repeat** and **lock-key state** (Caps Lock, Num Lock) from the keyboard feature;
- **a full-screen `xdg_toplevel` surface** in the `wayland` feature, as well as layer surfaces;
- **wallpaper scaling** once per output size (scootbg's `pic-scale-safe` path, or
  `scootui`'s triangle filter for a one-time downscale);
- the same lifecycle across output hotplug as the bar.

A future `scootlock` (`site/OUTLINE.md`, Coming apps) needs exactly the entry, focus and
lock-key pieces plus `ext-session-lock-v1`, so this widget set is built once for both.

**Budgets** (set against `greet-baseline`'s numbers when measured): RSS at most 8 MB with a
4K wallpaper decoded once and dropped after scaling; first frame at most 100 ms from process
start; a stripped binary at most 2 MB; closure glibc, libgcc_s, fonts and the wallpaper.

## Stylix and look integration

The login screen runs before any user session, as the `greeter` user, from `/etc`, so it is
**static** under the live-looks contract (§6.4; [shell-apps.md](shell-apps.md#theme-the-live-looks-contract-stylix-and-looks)). It reads
`/etc/scoot/look/greet.toml` (the contract renders it at system build time from
`programs.scoot.greeter.look`, which defaults to the declared look) merged **under**
`/etc/scoot/greet.toml` per key. It never reads Stylix: Stylix arrives as the `stylix`
look when that is the declared one. A live swap never touches the login screen.

- **Applying a look to the greeter** is a root-level step: `nixos-rebuild switch`, or on a
  non-Nix box `sudo scoot look apply --greeter NAME` (#578's `look-greeter` ticket, which
  renders `greet.toml` for scootgreet as it renders `regreet.css` for ReGreet). greetd has
  `restartIfChanged = false`, so a running greeter shows the new look from its next start
  (the next logout), and the docs say so.
- **The loader** takes allowlisted keys only (colors, radius, fonts, wallpaper path,
  cursor), with every string through the contract's grammar. A fragment that fails either
  is ignored with an error line on screen, and the built-in defaults draw ([Failure
  modes](#failure-modes-never-strand-a-login)). A look can never stop a login.
- **NixOS module** `programs.scootgreet` (NixOS only): `enable`, `package`, `settings`
  (TOML into `/etc/scoot/greet.toml`, mode 0644). There is no `stylix.enable`.
  `programs.scoot.greeter` gains `daemon = enum [ "regreet" "scootgreet" ]`, default
  `"regreet"`. With `"scootgreet"`, the module:
  - points greetd's `default_session.command` at `scootgreet-session`;
  - creates `/var/lib/scootgreet` with tmpfiles;
  - passes the XKB environment;
  - keeps the GDM/SDDM refusals and the forced scoot session entry (`nixos.nix:54-78`);
  - keeps the Stylix regreet-target assertion (`nixos.nix:86-88`) for ReGreet only.

| Element | Role | Through the `stylix` look | Shipped looks, today in ReGreet |
|---|---|---|---|
| Backdrop | `[wallpaper] image`, else `background` | `stylix.image`, else `base00` | `wallpaper.image` (`greeter.background`, `nixos.nix:2445-2447`) |
| Card fill | `surface` | `base01` | the look's `regreet.css` card |
| Labels, entry text, clock | `foreground` | `base05` | CSS |
| Placeholder, hints, info messages, date | `dim` | `base03` | CSS |
| Card border | `focus` | `base0D` | CSS |
| Focus ring, caret | `accent` | `base0D` | GTK focus |
| Primary button, selected user or session | `accent` / `accent-fg` | `base0D` / `base00` | `button.suggested-action` in the CSS |
| Auth errors, Caps Lock warning, destructive confirm | `urgent` | `base08` | `button.destructive-action` in the CSS |
| Font | `fonts.sans` at 12 pt unless `[apps.scootgreet]` sets a size | `sansSerif` | `fonts.sans` 12 pt (`nixos.nix:2456-2460`) |
| Cursor | `[cursor]` | `stylix.cursor` | Adwaita: the look's cursor never reached ReGreet (`theme-look.nix:61-69` vs no `cursorTheme` set) |

- **Tests** (`nix/scootgreet-tests.nix`): the rendered `greet.toml` equals the user's
  `settings`; `/etc/scoot/look/greet.toml` comes from `greeter.look`; the greetd command
  is the wrapper, and the agreety fallback is in it; tmpfiles; XKB env; `daemon = "regreet"`
  leaves today's output byte-identical (an eval pin against `_greeterPins`,
  `nix/tests.nix:6087-6310`); `--check` on the rendered files.
- **Without Nix**: `/etc/scoot/greet.toml` by hand, the look through the root-level apply
  step, greetd's `/etc/greetd/config.toml` pointing `default_session.command` at
  `scootgreet-session`, and the VT-escape note beside it. `install.sh` never installs the
  greeter ([shell-apps.md, Without Nix](shell-apps.md#without-nix)).

## The installer ISO

`scoot-sh/scoot-iso` (a separate repo; HEAD `75994a8`, `nix/live.nix:342-352`) uses greetd's
`initial_session` for live-media autologin with `agreety` as the `default_session`, and
`programs.scoot.greeter.enable` with ReGreet and the `moonrise` look for the installed
target (`nix/target-machine.nix:202-252`; not re-read for this doc). The ISO is 2.83 GiB, over GitHub's 2 GB asset cap
(`docs/backlog/core/iso-slim-under-2gb.md:15-19`). Whether the ReGreet stack is on the ISO
image itself (rather than fetched at install) is **not verified**. `greet-packaging-iso`
measures the ISO and target closure delta of switching the target to scootgreet, and
moves `scripts/qemu-test.sh` from coordinate clicks to typing into the focused
field. The live session keeps `agreety` and autologin unchanged.

## Testing and verification

- **Unit**: the conversation state machine against every response kind and order,
  including an info-only flow, two secret prompts (password change), and an error mid-flow;
  frame encoding and decoding with bounds; the password frame's escaping (quotes, backslashes,
  control characters, multibyte UTF-8) and its zeroization (a test reads the buffer after
  clearing); the passwd and login.defs parsers; session discovery; state file round-trip and
  corruption.
- **Fuzz**: greetd frames (length and JSON), `/etc/passwd` lines, and session desktop files (shared
  with the launcher's target).
- **A fake greetd** in tests (a Unix socket speaking the protocol with scripted PAM
  conversations), driving the real binary under `scoot --headless`. Input comes through scoot
  IPC `type` and `key`, and screenshots are asserted per state: user list, password with Caps Lock,
  OTP prompt, error, power confirm.
- **Real greetd on the dev VM**: a real PAM password login into a scoot session; a wrong password;
  `scootgreet login` headless; and a crash injected (`SCOOTGREET_TEST_ABORT=1`) to prove the agreety
  fallback and the absence of a dead VT1. **GPU-free**: the dev VM with no GPU, the host's
  software path.
- **M2**: a real login per look, the logind power buttons, a VT switch away and back, and a
  handoff into the scoot session with `live: true` on every output afterwards
  (`scoot msg outputs`).
- **Goldens** per shipped look and for a `stylix` look, for each state above.

## Docs and packaging

- The site's greeter section (`site/src/content/docs/desktop/index.md`, "The greeter")
  gains the choice of greeter, the recovery path, and a screen-reader note pointing at ReGreet.
  `start/compose.mdx`'s "a scoot greeter *(planned)*" becomes real. Its own reference pages
  (configure, `scootgreet login`, troubleshooting with `> **Symptom:**` boxes such as "the
  login screen is a text prompt", meaning the fallback ran, read the journal) go under a
  section added to `site/OUTLINE.md`'s Coming apps in the PR that ships it.
- Packages: deb, rpm and Arch, with a greetd config example and the `scootgreet-session` wrapper
  (`agreety` comes with greetd). Flake `packages.scootgreet` and `nixosModules.scootgreet`.
- CI: classify and job in `greet-greetd-client`, which creates the crate.

## Decisions specific to the greeter

These are also listed in [shell-apps.md](shell-apps.md#decisions-needed) items 2 and 9:

1. **Name**: `scootgreet` (recommended).
2. **Host**: cage first, scoot kiosk later and opt-in (recommended).
3. **Screen readers**: accept that scootgreet has none, keep ReGreet a supported option
   indefinitely, and document which to choose (recommended). The alternative is AT-SPI in
   `scootui`, a large separate project.
4. **Users from `/etc/passwd`**, with NSS-only users typing their name (recommended), over
   AccountsService over D-Bus.

## Tickets

| Slug | Serves | Blocked by |
|---|---|---|
| [`scootgreet`](../../docs/scootbar/backlog/scootgreet.md) (epic) | daily-drive, computer use (scriptable VM login) | its children |
| [`greet-baseline`](../../docs/scootbar/backlog/greet-baseline.md): measure ReGreet + cage; verify cage's exit status, its GPU-free path, and tty2's getty | daily-drive | go-ahead |
| [`greet-greetd-client`](../../docs/scootbar/backlog/greet-greetd-client.md): frames, conversation, secret path, `scootgreet login`, fake greetd | computer use, daily-drive | `greet-baseline`, `scootui-extract-draw` |
| [`greet-ui`](../../docs/scootbar/backlog/greet-ui.md): the screen, users, sessions, power, layout and Caps Lock, built-in fallback theme; the go/no-go gate | daily-drive | `greet-greetd-client`, `scootui-entry-and-list`, `scoot-dbus-extract`, `launch-desktop-index` |
| [`greet-nixos`](../../docs/scootbar/backlog/greet-nixos.md): module, the static look, wrapper with the agreety fallback, opt-in | daily-drive | `greet-ui` |
| [`greet-packaging-iso`](../../docs/scootbar/backlog/greet-packaging-iso.md): packages, docs, the scoot-iso switch and its QEMU test | daily-drive | `greet-nixos` |
| [`greet-default-flip`](../../docs/scootbar/backlog/greet-default-flip.md) | daily-drive | `greet-packaging-iso` |
| [`greet-scoot-kiosk-spike`](../../docs/scootbar/backlog/greet-scoot-kiosk-spike.md) (research) | daily-drive | `greet-nixos` |
| [`scoot-greeter-mode`](../../docs/backlog/core/scoot-greeter-mode.md) (core): the kiosk flags | daily-drive | `greet-scoot-kiosk-spike` |
