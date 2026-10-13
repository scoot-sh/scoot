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
  (nixpkgs `regreet.nix` L158-159). Measured by the #579 review at the pinned nixpkgs
  (`8ce4ef6c`, aarch64-linux, `nix path-info -S` against cache.nixos.org):
  ReGreet alone is 956.7 MiB of closure, and ReGreet with cage is 1,018.5 MiB (307 paths).
  **cage stays the host**, so what scootgreet can remove is only what ReGreet adds beyond
  cage: **638.4 MiB (166 paths; cage itself is 380.2 MiB)**, re-measured in the #579
  re-review. That is the ceiling: it is the package union, it leaves out greetd, dbus and
  accounts-daemon, and it is not yet net of the GTK and GStreamer libraries an installed
  desktop profile already carries. The real saving is that marginal number, which may be much
  smaller. ReGreet's RSS and first frame are still unmeasured too; `greet-baseline` measures
  all three. scootgreet needs no session bus, no
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

**Go/no-go gate (`greet-prototype`), before any extraction.** The gate tests what can
actually fail, and states the closure case explicitly instead of assuming it. It is an out-of-workspace prototype that `#[path]`-includes scootbar's sources, the
pattern scootbar's fuzz crate already uses (`crates/scootbar/fuzz/Cargo.toml:28-65`), so no
extraction is spent on a no. It must show:

- against a **real greetd** on the dev VM, two cases: a wrong password and then the right
  one in the same greeter; and the greeter SIGKILLed with a secret prompt pending, which
  must land on a working agreety login ([The greetd client](#the-greetd-client));
- the full PAM matrix: password, wrong password, an expired-password change, an OTP second
  prompt (`pam_oath` or a test module), and an info-only message;
- every layout the keyboard spike passes, typed into the password field, Compose included;
- RSS and first frame against `greet-baseline`'s ReGreet numbers;
- **a weight case that is still real**: either the marginal closure over the *installed
  desktop profile* that scootgreet would remove is at least about 300 MiB (measured by
  `greet-baseline`), or the scoot-iso image is still over its size cap with ReGreet on it.

**When not to build it**, decided up front:

- the two real-greetd cases cannot be made green;
- the keyboard spike cannot type every layout the maintainer uses;
- `greet-baseline` shows the marginal closure over the installed desktop profile is under
  about 300 MiB (GTK4 is already there), and the ISO is not over its cap;
- scootlock is chosen first ([Decisions needed (d)](shell-apps.md#decisions-needed)).

On a no, ReGreet stays.

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
  is out of scope" (`nixos.nix:2335-2338`). This plan keeps cage as the host. A scoot host
  is considered **only if cage fails the GPU-free check** in `greet-baseline`.

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

**greetd keeps a failed session until someone cancels it.** This is the part the
`greetd_ipc` crate's docs get wrong ("the session is cancelled automatically on error",
`greetd_ipc/src/lib.rs:64-66`). At greetd **0.10.3**, the version this plan targets:

- An EOF on the greeter's connection returns with **no** cancel (`greetd/src/server.rs:73`,
  `Err(CodecError::Eof) => return Ok(())`). Only a codec or IO error cancels (`server.rs:289`).
- After an error, the session worker's last message is cached in `last_msg`
  (`greetd/src/session/interface.rs:159-165`), and nothing clears the context's
  `configuring` session.
- The next `create_session` then fails with "a session is already being configured"
  (`greetd/src/context.rs:185-186`).
- `cancel` is a no-op returning `Ok` when nothing is being configured (`context.rs:224-230`).
- greetd's own agreety sends `CancelSession` after every `Error` (`agreety/src/main.rs:113-117`).

So the client **sends `cancel_session` after every `error` response, and before every
`create_session`**, including the first one after connecting or reconnecting. When nothing is
pending this costs one round trip and does nothing; when something is, it is what makes the
next attempt possible. A fake greetd in the tests must model the retained `configuring`
state. One written from the man page would pass every test while the real one strands
the second login attempt, which is the "passes and reads clean" trap CLAUDE.md describes.

**Conversation state machine** (pure, in `src/greetd/conversation.rs`):

```
Idle --(user chosen + Enter)--> cancel_session; create_session{username}
  auth_message visible  -> show the prompt text over a visible entry; Enter -> post_auth_message_response{response}
  auth_message secret   -> show the prompt text over a masked entry; Enter -> post_auth_message_response{response}
  auth_message info     -> show the text (dim); post_auth_message_response{} at once
  auth_message error    -> show the text (urgent); post_auth_message_response{} at once
  success               -> start_session{cmd, env} -> success -> exit 0
  error auth_error      -> cancel_session; "Login incorrect" (or its description); clear the entry; back to the password step via cancel_session + create_session
  error error           -> cancel_session; show the description; back to Idle
  start_session error   -> cancel_session; show the description; back to Idle (another session can be chosen)
Escape at any step      -> cancel_session; back to Idle (not while a PAM module blocks: below)
(re)connect             -> cancel_session first, always
```

- Messages are never assumed in number or order, so OTP and password-change ("current, new,
  retype") flows work.
- **A fingerprint prompt cannot be cancelled from the greeter.** `pam_fprintd` sends an info
  message, then blocks the PAM stack until a finger or its own timeout. greetd's
  `get_question` holds the context's write lock across that wait (`context.rs:233-236`,
  `self.inner.write().await` around `get_state()`), so a `cancel_session` from this or any other
  connection waits until the module returns.
  - The UX consequence: while the sensor waits, Escape cannot go back, so the screen does not
    promise it can. It shows "Touch the sensor, or wait for it to time out" with Escape greyed.
  - The mitigation is `pam_fprintd`'s own timeout option, which the docs point at, plus
    Ctrl+Alt+F2. Power buttons still work, because they go to logind and not to greetd.
- While a request is in flight (PAM's delay after a wrong password is often 2 s), input is
  disabled and a "Checking…" line is shown. The loop stays responsive: the greetd socket is
  non-blocking in the `poll` set.
- **Invariant: scootgreet exits 0 only after `start_session` succeeded.** Every other exit
  (a fatal error, a lost compositor) is non-zero, so the supervisor's fallback catches it. A
  clean exit without a session would send greetd round its restart loop, which the fallback
  never sees ([Failure modes](#failure-modes-never-strand-a-login)).
- After `start_session` succeeds, **exit at once**: "The session will start after the
  greeter process terminates". greetd gives the greeter 5 s, then SIGTERMs every second, then
  SIGKILLs at 10 s (`greetd/src/context.rs:294-314`).
- A dropped connection leaves greetd's session in place (above). The greeter reconnects
  and cancels before anything else.
- Bounds: frames are at most 64 KiB (refuse a larger one, then reconnect and cancel). At most
  4 messages are kept on screen (the newest), each at most 512 bytes and stripped of control
  characters.

**`start_session`.** `cmd` is the chosen session's `Exec`, split by the Desktop Entry
quoting rules (the parser comes from scootlaunch's index; see [Architecture](#architecture-and-what-the-shared-crate-must-offer)).
`env` sets `XDG_SESSION_TYPE=wayland`, `XDG_SESSION_DESKTOP` (the first of the entry's
`DesktopNames`, else the file id) and `XDG_CURRENT_DESKTOP` (`DesktopNames` joined by
`:`). The scoot session launcher already handles a login that "arrives with `wayland`
already" (`resources/scoot-session:474-475`).

**`scootgreet login` (headless).** For VM tests and agents: `scootgreet login --user U
[--session NAME]`. It runs the same state machine against `$GREETD_SOCK`, with the same
cancels. It answers **every** prompt, `visible` and `secret` alike, with the next line read
from stdin (or from `--answers-fd N`), and prints `info` and `error` messages to stderr. It
exits 0 once the session is started. No answer ever goes in argv: a `visible` answer can be an
OTP, and argv is readable by any local user through `/proc/PID/cmdline` unless `/proc` is
mounted with `hidepid`.

**A headless login does not need the graphical greeter.** Anything running as the greeter user
with `GREETD_SOCK` can drive greetd, so the computer-use case (an agent or a VM test logging in)
does not have to wait for scootgreet. The frame codec, the conversation with its cancel
discipline, and the stdin answers can ship first as a small Wayland-free tool:
[`greet-headless-login`](../../docs/scootbar/backlog/greet-headless-login.md), a candidate early
ticket. Its crate is the one that later grows the UI, and `scootgreet login` is that tool. It has to run as the greeter user with the greeter's socket, exactly
No new privilege is introduced.

## What the screen offers

**Layout.** A full-screen `xdg_toplevel` asks for fullscreen and draws the background (the look's
wallpaper, else the `background` token) with a centered card holding:

- a clock;
- the user (a list when several; the last user preselected; "Other…" to type a name);
- the PAM prompt and its entry, focused at once;
- the session selector (the last session for that user preselected);
- a message line;
- power buttons (Suspend, Reboot, Power Off) with a confirm step, as scootbar's power menu has.

An `xdg_toplevel` is used rather than a layer surface: cage 0.3.1 has no layer-shell (the
#579 review grepped cage at `v0.3.1`), and an `xdg_toplevel` runs under cage, scoot and sway
alike.

**Keyboard-only flow.** Tab and Shift+Tab move through user, entry, session and power. Up and Down
change the user or the session. Enter submits. Escape cancels a conversation. The pointer works
too. The focus ring is the `accent` token, 2 device pixels.

**Users.** Read `/etc/passwd` directly (no NSS, no `libc` FFI). Keep UIDs in
`UID_MIN`..`UID_MAX` from `/etc/login.defs` (defaults 1000 and 60000, the range tuigreet
uses), drop shells ending in `nologin` or `false` (an **empty** shell field means `/bin/sh`
and is kept), and apply config `hide-users` / `show-users`. **NSS-only users** (LDAP, sssd, systemd-homed) are not listed. They use "Other…"
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
`default-session` (the profile sets `"scoot"`), else the first. **No sessions found** (an
empty or unreadable data path): the selector shows "No sessions installed". The
config's `fallback-session` argv is used if set; the NixOS module sets it to the scoot
session's `Exec`, and it is the same command the supervisor hands agreety as `--cmd`.
Otherwise, Login is disabled and the message names the directories searched. The
supervisor's agreety fallback stays available either way.

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

**Strings** are English in v1, which is a stated regression risk: whether ReGreet localizes its
labels is not verified. A message catalog (gettext-free, a TOML table per locale chosen by
`LANG`) is a follow-up. PAM's own messages arrive already localized by PAM.

**The screen never blanks.** cage has no output-power support, so a login screen left alone
stays lit. This is stated in the docs. A blanking timer through scoot's output power is
possible only with the scoot host.

**Outputs.** The card goes on one output: config `output` (a name), else the output with the
pointer, else the first. Every other output gets the background. Hotplug moves the card if its
output leaves. Fractional scale comes through `wp-fractional-scale-v1` + `wp-viewporter`, as the bar. Under
cage with `-m last` there is one output. Under scoot every output is used.

## Which compositor hosts it

| Host | For | Against |
|---|---|---|
| **cage** (the host) | Proven here; exits when its app exits and **returns the app's exit status** (cage 0.3.1, `cage.c:111` sets `return_app_code` when the client goes away, `cage.c:658-660` returns it); `-s` allows VT switching (`cage.c:278-279`); tiny; already in the NixOS module | A second compositor (wlroots) in the closure; its software-rendering path on a GPU-less box is **not verified** (`greet-baseline` checks it on the dev VM, CLAUDE.md's GPU-free rule). If cage fails there it exits non-zero, and agreety takes over |
| **scoot in a kiosk mode** (only if cage fails the GPU-free check) | One compositor in the closure; pixman by default, so GPU-free by construction; the same cursor and output handling as the session | Real core work and a new compositor attack surface at the login screen ([below](#what-scoot-would-need)); a scoot bug then also takes out the login screen. Whether to add that surface at all is the maintainer's call |

**Command**: `cage -s -d -m last -- scootgreet`, run by the supervisor
([Failure modes](#failure-modes-never-strand-a-login)). There is no `dbus-run-session`:
scootgreet uses only the system bus, which removes the leak class without the
`KillOnlyUsers` workaround (the workaround stays for ReGreet users).

### What scoot would need

Spike `greet-scoot-kiosk-spike` (research), then core ticket `scoot-greeter-mode`. Both
are blocked on cage failing the GPU-free check in `greet-baseline`, and nothing starts them
otherwise. Everything below is verified absent today:

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

If it is ever built, it is opt-in (`programs.scoot.greeter.host = "scoot"`).

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
  - Keysyms become UTF-8 **into a caller-supplied buffer** inside the locked page, never through
    an API that returns a `String` (the `xkbcommon` crate's `key_get_utf8` does), which would
    leave copies of typed characters in freed heap. The keyboard spike records which call each
    candidate offers.
  - **Locked memory.** Both buffers live in one small locked region (two pages): mapped,
    `mlock`ed, `MADV_DONTDUMP`, and zeroed and unlocked on drop. In rustix 1.1, `mlock` is an
    `unsafe fn` (`src/mm/mmap.rs:312` in 1.1.5), so the region is a new type in `scootbg-mem`,
    the family's one audited `unsafe` crate. It sits **behind a `locked` feature that is off by
    default**, so scootbg's release-gated binary never builds it, and the PR that adds it updates
    the crate's description, which today speaks of wallpaper buffers only. scootgreet keeps
    `forbid(unsafe_code)`.
    **Not `mlockall(MCL_FUTURE)`**, although rustix offers it safely (`mmap.rs:408`): under an
    unprivileged `RLIMIT_MEMLOCK` of 8 MiB, with an 8 MB RSS budget, it would turn ordinary
    allocations into failures, which means an abort at the login screen. (greetd itself
    `mlockall`s as root, `greetd/src/main.rs:44`.) `memfd_secret` is the stronger alternative
    where the kernel enables it, and the spike notes whether the M2 and dev VM kernels do.
  - **Hibernation writes locked pages to disk.** Locking keeps the secret out of swap, not out of
    a hibernation image; the docs say so.
  - **`PR_SET_DUMPABLE` 0 at start** (rustix's safe `set_dumpable_behavior`,
    `src/process/prctl.rs:140`). It buys two things: no core dump, and no `ptrace` or
    `/proc/PID/mem` access from other processes of the same uid. cage runs as the same `greeter`
    uid, so that matters here.
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
2. **A supervisor, `scootgreet session`, runs the graphical greeter.** It is a code path in the
   same binary with no Wayland, fonts or drawing. It:
   1. sends `cancel_session` (a stale session from a previous greeter must not block the next
      one; [above](#the-greetd-client));
   2. starts `cage -s -d -m last -- scootgreet` in its own process group, by absolute store path,
      with a pipe whose write end scootgreet inherits;
   3. requires one byte on the pipe within a **startup deadline** (config `startup-timeout`,
      default 15 s), written after scootgreet's first frame is presented. `greet-baseline`
      measures the first frame, including the slow case: QEMU without KVM and with no GPU, the
      scoot-iso test's setup. The default is set from that number with a margin;
   4. after that, requires a heartbeat byte at least every **90 s**. scootgreet's loop writes one
      every 30 s from a `timerfd`: two wakeups a minute at the login screen, an accepted and
      stated cost;
   5. falls back when cage exits non-zero (cage returns the app's status, above), or a deadline
      passes (the greeter hung rather than crashed). It SIGKILLs the process group, sends
      `cancel_session` again, and execs **agreety** by absolute path:
      `agreety --cmd <fallback-session>`.

   **Both timers are `CLOCK_MONOTONIC`.** Linux's monotonic clock does not advance while the
   machine is suspended (`CLOCK_BOOTTIME` does), so a laptop that sleeps at the login screen and
   resumes does not trip the heartbeat. The supervisor's own waits are `poll` timeouts, which
   are measured on the same clock, and scootgreet's heartbeat `timerfd` is created on
   `CLOCK_MONOTONIC` explicitly.

   **Every fallback says why.** Before exec'ing agreety, the supervisor:
   - prints one line to the VT, which agreety then shares, for example
     `scootgreet: no first frame within 15 s; text login follows`, or
     `scootgreet: exited with status 134 (SIGABRT)`;
   - appends the same line, timestamped, to `/var/lib/scootgreet/fallback.log` (kept to its last
     64 lines), so the reason survives for a bug report.

   agreety is greetd's own text greeter, and `scoot-iso`'s live `default_session`
   (`scoot-iso` `nix/live.nix:342-352`). A greeter that cannot draw, crashes, or hangs
   degrades to a text login on the same VT, never to a dead VT.
3. **If the supervisor itself fails, a shell wrapper catches it.** greetd does not run its
   command directly. It builds `exec <command>` (`greetd/src/session/worker.rs:228-235`) and
   runs that with `/bin/sh -c` (`worker.rs:267-272`). So a `default_session.command` of
   `scootgreet session || fallback` would really be `exec scootgreet session || fallback`.
   - If the supervisor starts, the shell is already replaced, and `||` never runs.
   - If the exec fails (a missing or broken binary), a non-interactive shell exits at once.

   Either way the fallback is unreachable. A supervisor that dies on every start would then
   loop greetd into systemd's start limit and leave VT1 dead. **greetd's command is therefore
   one executable `sh` wrapper, `scootgreet-greeter`**, so that greetd's `exec` replaces the
   shell with the wrapper, not with the supervisor:

   ```sh
   #!/bin/sh
   # scootgreet-greeter: greetd's default_session.command (greetd execs it under sh -c).
   /abs/bin/scootgreet session && exit 0
   status=$?
   # The wrapper logs the supervisor's status itself: the supervisor may be what failed.
   # A failed write must never block the fallback, hence the `|| :` and no `set -e`.
   printf '%s scootgreet session exited %s\n' "$(date -u +%FT%TZ)" "$status" \
       >>/var/lib/scootgreet/fallback.log 2>/dev/null || :
   exec /abs/bin/scootgreet-fallback
   ```

   `scootgreet-fallback` is a second `sh` script. **The session command is baked into it at build
   or install time.** It cannot come from `greet.toml`'s `fallback-session`, because only the
   supervisor reads that file, and the fallback runs exactly when the supervisor has failed.
   With an empty or unset variable, agreety would get `--cmd ""`: getopts' `opt_default` gives
   `Some("")` (`agreety/src/main.rs:139,159`), greetd runs `sh -c "exec "`, and the user bounces
   straight back. So:

   ```sh
   #!/bin/sh
   # Under Nix, @session@ is substituted with the scoot session's absolute Exec at build time.
   # Packages write it at install time (default: the scoot session's absolute Exec).
   session='@session@'
   if [ -n "$session" ]; then set -- --cmd "$session"; else set --; fi
   /abs/bin/agreety "$@" && exit 0
   exec /abs/bin/agreety "$@"
   ```

   With no session known, `--cmd` is left out entirely, and agreety prompts `Command:`
   (`agreety/src/main.rs:103-106`, when `cmd` is `None`). That beats a session that exits at once.

   It runs agreety again only when agreety exits non-zero. That is the stale-session case:
   agreety's first request meets a session a dead greeter left, gets an `error`, cancels it,
   and exits 1 (`agreety/src/main.rs:113-121,176-179`). The second run starts clean.
   **agreety exits 0 when it starts a session, and also when it has used up its
   `--max-failures` (default 5) wrong passwords** (the loop at `main.rs:173-182` ends and
   `main` returns). So a zero exit is left to greetd: with no session started, greetd sees the
   greeter exit, exits 0, and systemd restarts it (`Restart=on-success`). Each such cycle waits
   for a person typing at agreety, so it cannot spin into the start limit.

   A `scootgreet` that crashes deterministically on some PAM flow (OTP, say) therefore still
   ends at a working text login on every greeter start. So does a supervisor binary that is
   missing, exits non-zero at once, or crashes. That is the case R1 and N1 of the #579 reviews
   showed the earlier drafts got wrong.

   **The supervisor's only exit-0 path is cage exiting 0.** cage returns the app's status when
   the app goes away (`cage.c:111,658-660`), and scootgreet exits 0 only after `start_session`
   succeeded ([invariant](#the-greetd-client)). cage can also exit 0 on its own SIGINT or SIGTERM
   (`cage.c:223-230`), but that needs an outside signal each time, so it cannot loop by itself. Every other
   supervisor outcome is non-zero, including its own errors, a timeout, and a signal it
   reports. A buggy exit 0 without a session therefore cannot skip the fallback and push greetd
   towards its start limit.

   **The wrapper and the fallback must be executable.** A non-executable wrapper makes greetd's
   `sh -c "exec …"` exit 126 on every start, which loops greetd into its start limit: a dead
   VT1. The Nix tests and every package pin the mode (`0755`).

   **Not caught: a hung cage.** The heartbeat proves scootgreet's loop is alive, not that cage
   is putting frames on the screen. Tying it to presented frames would false-trip whenever the
   user switches to another VT, since no frames are presented there. A cage that hangs while
   scootgreet keeps beating leaves a frozen login screen. Recovery is `Ctrl+Alt+F2` (if cage still
   switches VTs), ssh, SysRq, or a hard reset. That residual risk is accepted and documented.
4. **VT escape.** `Ctrl+Alt+F2` reaches a getty on tty2: nixpkgs disables only `autovt@tty1`, and
   cage's `-s` allows the switch. `greet-nixos` verifies this on the dev VM, and the install docs
   print it next to the switch. **To verify:** after the supervisor SIGKILLs cage, whether the VT
   is back in text and keyboard mode before agreety runs (logind should restore it when the
   controlling session drops). That is a `greet-nixos` acceptance case.
5. **A greetd socket error or protocol error** shows "Login service unavailable" with the
   power buttons, and retries the connection every 2 s, cancelling first.

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
`scootui`. It lands in `greet-users-sessions`, which adds `scoot-xdg`'s CI classify arm
(fanning out to scootlaunch and scootgreet) and its CLAUDE.md scope entry.

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
**static** under the live-looks contract (#578 §6.4 and §9 item 7;
[shell-apps.md](shell-apps.md#theme-the-live-looks-contract-stylix-and-looks)). It reads the
`greet.toml` fragment that #578 renders at system build time from
`programs.scoot.greeter.look` (default: the declared look), merged **under**
`/etc/scoot/greet.toml` per key (#578 §4.3). This plan assumes the fragment lands at
`/etc/scoot/look/greet.toml`; the path itself is #578's `look-greeter` ticket's to fix. It
never reads Stylix: Stylix arrives as the `stylix` look when that is the declared one. A live
swap never touches the login screen.

- **Applying a look to the greeter** is a root-level rebuild, never a user swap. greetd has
  `restartIfChanged = false`, so a running greeter shows the new look from its next start (the
  next logout), and the docs say so. A non-Nix `sudo scoot look apply --greeter` is "later,
  not v1" in #578 §6.4; until then a non-Nix greeter takes its look from a hand-written
  `greet.toml`.
- **The loader** takes allowlisted keys only (colors, radius, fonts, wallpaper path,
  cursor), with every string through the contract's grammar. A fragment that fails either
  is ignored with an error line on screen, and the built-in defaults draw ([Failure
  modes](#failure-modes-never-strand-a-login)). A look can never stop a login.
- **NixOS module** `programs.scootgreet` (NixOS only): `enable`, `package`, `settings`
  (TOML into `/etc/scoot/greet.toml`, mode 0644). There is no `stylix.enable`.
  `programs.scoot.greeter` gains `daemon = enum [ "regreet" "scootgreet" ]`, default
  `"regreet"`. With `"scootgreet"`, the module:
  - sets greetd's `default_session.command` to the **`scootgreet-greeter` wrapper's absolute
    store path**, and nothing else, so greetd's `exec` lands on the wrapper ([why](#failure-modes-never-strand-a-login));
  - creates `/var/lib/scootgreet` with tmpfiles;
  - passes the XKB environment;
  - keeps the GDM/SDDM refusals and the forced scoot session entry (`nixos.nix:54-78`);
  - keeps the Stylix regreet-target assertion (`nixos.nix:86-88`) for ReGreet only.

| Element | Role | Through the `stylix` look | Shipped looks, today in ReGreet |
|---|---|---|---|
| Backdrop | `[wallpaper] image`, else `background` | `stylix.image`, else `base00` | `wallpaper.image` (`greeter.background`, `nixos.nix:2445-2447`) |
| Card fill | `surface` (default `background`) | `base00` | the look's `regreet.css` card |
| Labels, entry text, clock | `foreground` | `base05` | CSS |
| Placeholder, hints, info messages, date | `dim` | `base03` | CSS |
| Card border | `focus` | `base0D` | CSS |
| Focus ring, caret | `accent` | `base0D` | GTK focus |
| Primary button, selected user or session | `accent` / `accent-fg` | `base0D` / `base00` | `button.suggested-action` in the CSS |
| Auth errors, Caps Lock warning, destructive confirm | `urgent` | `base08` | `button.destructive-action` in the CSS |
| Font | `fonts.sans` at `fonts.app-size` (default 12 pt = 16 px) | `sansSerif` at `sizes.applications` | `fonts.sans` 12 pt (`nixos.nix:2456-2460`) |
| Cursor | `[cursor]` | `stylix.cursor` | Adwaita: the look's cursor never reached ReGreet (`theme-look.nix:61-69` vs no `cursorTheme` set) |

- **Tests** (`nix/scootgreet-tests.nix`): the rendered `greet.toml` equals the user's
  `settings`; the look fragment comes from `greeter.look`; the greetd command is the
  supervisor, and the agreety fallback is in it; tmpfiles; XKB env; `daemon = "regreet"`
  leaves today's output byte-identical (an eval pin against `_greeterPins`,
  `nix/tests.nix:6087-6310`); `--check` on the rendered files.
- **Without Nix**: `/etc/scoot/greet.toml` by hand, the look through the root-level apply
  step, greetd's `/etc/greetd/config.toml` pointing `default_session.command` at
  the `scootgreet-greeter` wrapper, and the VT-escape note beside it. `install.sh` never installs the
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
- **A fake greetd** in tests: a Unix socket speaking the protocol with scripted PAM
  conversations, **modelling greetd 0.10.3's retained `configuring` state**. An error or EOF
  leaves the session in place, and `create_session` fails until a `cancel_session`. It drives
  the real binary under `scoot --headless`. Input comes through scoot IPC `type` and `key`, and
  screenshots are asserted per state: user list, password with Caps Lock, OTP prompt, error,
  fingerprint wait, power confirm. The fake is checked against the real greetd by running the
  same two scripted flows on both.
- **Real greetd on the dev VM** (greetd 0.10.3):
  - a real PAM password login into a scoot session;
  - **a wrong password, then the right one, in the same greeter**;
  - **scootgreet SIGKILLed with a secret prompt pending**, landing on a working agreety login;
  - a hang injected (`SCOOTGREET_TEST_HANG=1` stops the heartbeat), landing on agreety
    after the deadline;
  - **the supervisor itself failing**, through the real `scootgreet-greeter` wrapper as greetd
    runs it: the `scootgreet` binary missing, `scootgreet session` exiting non-zero at once,
    and `scootgreet session` aborting. Each must reach a working agreety login **that starts
    the scoot session** (the baked-in command, not an empty one). The wrapper's own line in
    `fallback.log` records the exit status;
  - the same with `/var/lib/scootgreet` missing or read-only: the log write fails silently,
    and the fallback still runs;
  - a fallback built with no session command: agreety prompts `Command:` instead of bouncing;
  - agreety's `--max-failures` used up, ending in a fresh greeter (greetd's restart) rather
    than a dead VT;
  - a suspend and resume at the login screen, with no fallback;
  - cage itself SIGKILLed, with agreety's prompt readable afterwards;
  - `scootgreet login` headless, with its answers on stdin.
- **GPU-free**: the dev VM with no GPU, the host's software path.
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
- Packages: deb, rpm and Arch, with the `scootgreet-greeter` and `scootgreet-fallback`
  scripts and a greetd config example pointing `default_session.command` at
  `scootgreet-greeter` (`agreety` comes with greetd). Both scripts are installed mode `0755`;
  the fallback's session command is written at install time (default: the scoot session's
  absolute `Exec`); and `/var/lib/scootgreet` is created for the greeter user through
  `tmpfiles.d`, as the Nix module does. Flake `packages.scootgreet` and `nixosModules.scootgreet`.
- CI: classify and job in `greet-headless-login`, which creates the crate.

## Decisions specific to the greeter

These are also listed in [shell-apps.md](shell-apps.md#decisions-needed) items 2 and 9:

1. **Name**: `scootgreet` (recommended).
2. **Host**: cage. A scoot kiosk host only if cage fails the GPU-free check (recommended).
   Whether to add that compositor attack surface at all is the maintainer's call.
3. **Screen readers**: accept that scootgreet has none, keep ReGreet a supported option
   indefinitely, and document which to choose (recommended). The alternative is AT-SPI in
   `scootui`, a large separate project.
4. **Users from `/etc/passwd`**, with NSS-only users typing their name (recommended), over
   AccountsService over D-Bus.
5. **The fallback greeter: agreety** (recommended) over ReGreet. agreety is text-only, ships
   in greetd's own package, and needs no compositor, GPU or font. Those are the things whose
   failure the fallback exists to catch.
6. **The default flip must not regress screen-reader users at first boot** (recommended):
   `greet-default-flip` keeps ReGreet the default whenever the system enables a screen reader
   (the exact NixOS option is found and pinned in that ticket), and the scoot-iso installer
   offers the choice.

## Tickets

| Slug | Serves | Blocked by |
|---|---|---|
| [`scootgreet`](../../docs/scootbar/backlog/scootgreet.md) (epic) | daily-drive, computer use (scriptable VM login) | its children |
| [`greet-baseline`](../../docs/scootbar/backlog/greet-baseline.md): measure ReGreet + cage (RSS, first frame, the marginal closure over the profile); verify cage's GPU-free path and tty2's getty | daily-drive | none: start now |
| [`greet-prototype`](../../docs/scootbar/backlog/greet-prototype.md): the go/no-go gate, out of the workspace, against a real greetd | daily-drive | `greet-baseline`, `scootui-keyboard-spike`, the go-ahead |
| [`greet-headless-login`](../../docs/scootbar/backlog/greet-headless-login.md): the greetd client, the cancel discipline and `scootgreet login`, with no UI (a candidate early ticket for computer use) | computer use | the go-ahead |
| [`greet-greetd-client`](../../docs/scootbar/backlog/greet-greetd-client.md): the supervisor and its wrapper, the fallback, the locked secret path | daily-drive | `greet-headless-login`, the GO verdict in `greet-prototype`, `scootui-extract-draw` |
| [`greet-ui`](../../docs/scootbar/backlog/greet-ui.md): the screen, entry, focus chain, layout and Caps Lock, built-in defaults | daily-drive | `greet-greetd-client`, `scootui-entry-and-list`, `scootui-surfaces-on-demand` |
| [`greet-users-sessions`](../../docs/scootbar/backlog/greet-users-sessions.md): `/etc/passwd`, avatars, sessions through a new `scoot-xdg` crate, remembered state | daily-drive | `greet-ui`, `launch-desktop-index` |
| [`greet-power`](../../docs/scootbar/backlog/greet-power.md): logind power with `Can*` gating | daily-drive | `greet-ui`, `scoot-dbus-extract` |
| [`greet-nixos`](../../docs/scootbar/backlog/greet-nixos.md): module, the static look, the supervisor and agreety fallback, opt-in | daily-drive | `greet-users-sessions`, `greet-power`, `look-format` |
| [`greet-packaging-iso`](../../docs/scootbar/backlog/greet-packaging-iso.md): packages, docs, the scoot-iso switch and its QEMU test | daily-drive | `greet-nixos` |
| [`greet-default-flip`](../../docs/scootbar/backlog/greet-default-flip.md) | daily-drive | `greet-packaging-iso` |
| [`greet-scoot-kiosk-spike`](../../docs/scootbar/backlog/greet-scoot-kiosk-spike.md) (research) | daily-drive | cage failing the GPU-free check in `greet-baseline` |
| [`scoot-greeter-mode`](../../docs/backlog/core/scoot-greeter-mode.md) (core): the kiosk flags | daily-drive | `greet-scoot-kiosk-spike` |
