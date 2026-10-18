# scootlaunch: design

Design pass, 2026-10-12. This covers what the launcher does and how it is built. What it
shares with scootnotify and scootgreet (`scootui`, `scoot-dbus`, the keyboard
stack, the live-looks theming contract, CI and start order) is in [shell-apps.md](shell-apps.md). The
tickets are under [the epic](../../docs/scootbar/backlog/launcher.md). The
name is `scootlaunch`, not `scootlauncher` ([shell-apps.md, Names](shell-apps.md#names)).

Spec references:

- Desktop Entry Specification **1.5**
  (<https://specifications.freedesktop.org/desktop-entry/latest/>);
- XDG Base Directory **0.8**
  (<https://specifications.freedesktop.org/basedir/latest/>);
- Icon Theme Specification **0.13**
  (<https://specifications.freedesktop.org/icon-theme/latest/>);
- `xdg-activation-v1` (wayland-protocols, staging);
- systemd's desktop-environment conventions (<https://systemd.io/DESKTOP_ENVIRONMENTS/>).

## Contents

1. [Goals, non-goals and the case for it](#goals-non-goals-and-the-case-for-it)
2. [Modes](#modes)
3. [The desktop entry index](#the-desktop-entry-index)
4. [Exec: from a desktop entry to an argv](#exec-from-a-desktop-entry-to-an-argv)
5. [Matching and ranking](#matching-and-ranking)
6. [Keyboard-first UX](#keyboard-first-ux)
7. [Launching correctly](#launching-correctly)
8. [The dmenu contract](#the-dmenu-contract)
9. [The agent interface](#the-agent-interface)
10. [Architecture](#architecture)
11. [Security](#security)
12. [Failure modes and edge cases](#failure-modes-and-edge-cases)
13. [Stylix and look integration](#stylix-and-look-integration)
14. [Testing and verification](#testing-and-verification)
15. [Docs and packaging](#docs-and-packaging)
16. [Tickets](#tickets)

## Goals, non-goals and the case for it

**Goals.** A keyboard-first application launcher that opens in a frame, launches
apps the way a desktop should (activation token, own systemd scope), and holds nothing
when closed. A dmenu mode that the bar's pickers and the user's scripts drive
unchanged. A JSON interface an agent can use without the UI.

**Non-goals (v1)**, so their absence is a decision:

- Clipboard history, emoji, calculator, file search, web search, and plugins. A clipboard
  picker already exists as a pipe into the dmenu mode
  (`cliphist list | fuzzel --dmenu …`, `nix/modules/keys-home.nix:103-118`), and
  that pipe keeps working. An emoji picker is the same shape over an emoji list.
  scootlaunch ships no plugin ABI, matching scootbar's "no dylib plugins, no embedded
  scripting" (`docs/scootbar/backlog/README.md`, Decisions).
- IME in the search field (v1). It is a follow-up through `text-input-v3`
  ([shell-apps.md, Keyboard](shell-apps.md#keyboard-and-text-entry)).
- A resident daemon. The pointer says it must "hold nothing when closed"
  (`docs/scootbar/backlog/launcher.md:29-32`).
- `%f/%F/%u/%U` with real files. A launcher opens apps, not documents. The codes are
  removed as the spec allows: "If the application should not open any file the %f,
  %u, %F and %U field codes must be removed".

**What we gain over the incumbent** (fuzzel through `scoot-launcher`;
`docs/backlog/resolved/desktop-launcher-done.md`):

- *Cost.* fuzzel is 152.4 MiB closure (41.7 MiB marginal), about 22 MB RSS open with icons
  (the #579 review measured fuzzel 1.14.1 open with an empty `$HOME` on the M2 at 18,944 kB
  RSS, 15,951 kB PSS and 17 threads), and
  a 50 to 57 ms cold first frame. scootlaunch targets the bar family's libc-only closure
  (plus `resvg` if SVG icons ship), under 8 MB open, and under 35 ms ([budgets](#budgets)).
- *Precedence done right.* Today the look's seven fuzzel color flags beat the
  user's own `fuzzel.ini` for those colors ("A color set in both places loses to the
  look", `site/src/content/docs/desktop/index.md:757-762`), because CLI flags beat
  fuzzel's file. With a config file that the Nix module writes per leaf, the user wins again.
- *Activation that works for running apps.* fuzzel mints its activation token
  from its keyboard *focus* serial when an entry is picked with the mouse, and scoot
  refuses that token (`crates/scoot/src/compositor/activation.rs:81-99`). For a
  single-instance app that is already running, "nothing visible happens".
  scootlaunch mints from the key or button press that chose the entry.
- *Apps in their own scope.* Today an app started from a scoot bind lives in
  `scoot.service`'s cgroup (scoot spawns with a plain `Command`,
  `crates/scoot/src/compositor/state.rs:1733-1803`, and its unit sets no `KillMode`),
  so it dies with the compositor unit and has no per-app accounting.
  scootlaunch puts each launch in `app-scootlaunch-<id>-<random>.scope`.
- *Computer use.* `scootlaunch --list --json` and `--run ID` need no compositor
  connection, rank like the UI does, and use the same spawn path, so an agent launches
  apps without screenshots.
- *Look consistency*: the same text, tokens and corners as the bar and the notifications.

**What we lose:** fuzzel's text stack (fcft, which can shape through HarfBuzz;
not re-checked for the nixpkgs build), its SVG icons, its many options, and its
maturity.

**Go/no-go gate (`launch-prototype`), before any extraction.** An out-of-workspace prototype
`#[path]`-includes scootbar's sources (the pattern `crates/scootbar/fuzz/Cargo.toml:28-65` uses)
and the keyboard spike's pick. It must show:

- first frame warm and cold, open RSS, and keystroke-to-commit at 300 and 3,000 entries, all at or
  under fuzzel's in the same harness on the M2;
- a query typed correctly on AZERTY and `us(intl)` dead keys;
- a mouse-picked entry re-activating an already-running single-instance app, the case scoot's
  own source documents fuzzel failing (`crates/scoot/src/compositor/activation.rs:81-99`).

**Verdict: GO, measured 2026-10-18** (record in
[`launch-prototype.md`](../../docs/scootbar/backlog/resolved/launch-prototype-done.md),
screenshots in `dev/spikes/scootbar/launch-prototype/`).
Dev-VM-only rows (no M2 access that session): first frame 25-37 ms vs
fuzzel's 64-78 ms at 300 and 3,000 entries; open RSS 4.9/6.2 MB vs
~14 MB; keystroke-to-pixels tied at 25-37 ms; AZERTY and `us(intl)`
queries correct; button-serial token re-activates the running app
while a focus-serial token is refused, the fuzzel failure shape.
Identical-list stress shows a 47 ms desktop-parse cost at 3,000 files,
which the planned index cache removes.

The RSS and closure rows are close to certain to pass. Keystroke latency and typing
correctness are the real risks. On a no, fuzzel stays, and nothing has been extracted for it.

## Modes

| Invocation | Mode |
|---|---|
| `scootlaunch` | apps (desktop entries plus their actions) |
| `scootlaunch --run` (alias `--list-executables-in-path`, fuzzel's flag) | executables on `$PATH` |
| `scootlaunch --dmenu [flags]` | lines on stdin, the pick on stdout ([contract](#the-dmenu-contract)) |
| `scootlaunch --windows` | open windows; Enter activates ([below](#windows-mode)) |
| `scootlaunch --list [--json]`, `--query TEXT [--json]`, `--run-id ID [--action A]` | headless agent interface ([below](#the-agent-interface)) |
| `scootlaunch --check [--config PATH]` | validate the config and exit (as `scootbar daemon --check`) |

### Windows mode

It lists **and** activates through `wlr-foreign-toplevel-management`: each
`zwlr_foreign_toplevel_handle_v1` carries `title`, `app_id` and `state`, and its `activate(seat)`
"hands the keyboard on like a click" (`site/src/content/docs/scoot/protocols.md:1310-1330,1393-1398`).
`ext-foreign-toplevel-list-v1` is not used. It is enumeration only, and its handles share no
handle or identifier with wlr's (scoot describes "one window list described twice", not a
mapping), so a window listed from one protocol could not be activated through the other. The
mode works on sway too. The brief
suggested scoot's IPC `windows` list. That is rejected under CLAUDE.md's
standard-protocol rule: the protocol already carries what this mode needs, and IPC would
tie the mode to scoot. Window icons come from matching `app_id` to a desktop
file id, or to `StartupWMClass`.

## The desktop entry index

**Where.** `$XDG_DATA_HOME/applications` (default `~/.local/share`), then each
`$XDG_DATA_DIRS` entry's `applications` (default `/usr/local/share/:/usr/share/`).
Relative entries are ignored ("must be absolute … consider the path invalid and ignore it",
basedir). Subdirectories are walked, because they form ids.

**Ids and precedence** (Desktop Entry §2.1): the path relative to the data
dir, minus `applications/`, with `/` turned into `-`. "If multiple files have the same
desktop file ID, the first one in the $XDG_DATA_DIRS precedence order is used", with
`XDG_DATA_HOME` ahead of all. `Hidden=true` makes the id not exist ("strictly
equivalent to the .desktop file not existing at all"), so a user-level `Hidden` file
masks a system entry, because the user's directory is searched first. The spec leaves
`foo-bar.desktop` beside `foo/bar.desktop` undefined. The first one found wins, and a
debug line records it.

**Which entries show.** Only `Type=Application`; unknown types are ignored, as the spec
says. `NoDisplay=true` entries are indexed but hidden from the list (they are still
reachable by `--run-id`, since "this application exists"). For `OnlyShowIn` /
`NotShowIn` against `$XDG_CURRENT_DESKTOP` (colon-separated, considered in order),
scoot sets that variable for its children (`state.rs:1831-1841`). `TryExec` is looked up
on `$PATH` at index time. A missing or non-executable target hides the entry.

**Which keys are read.** `Name`, `GenericName`, `Comment`, `Keywords`, `Icon`, `Exec`,
`TryExec`, `Path`, `Terminal`, `Actions` and the `Desktop Action` groups,
`DBusActivatable`, `StartupWMClass`, `NoDisplay`, `Hidden`, `OnlyShowIn`, `NotShowIn`
and `StartupNotify`. `PrefersNonDefaultGPU` and `SingleMainWindow` are "only a hint"
and are ignored, which is stated. `Categories` and `MimeType` are not used in v1.

**Localized keys** (§5): the locale comes from `LC_ALL`, then `LC_MESSAGES`, then `LANG`.
`.ENCODING` is stripped, and the lookup order is the spec's table:
`lang_COUNTRY@MODIFIER`, `lang_COUNTRY`, `lang@MODIFIER`, `lang`, then the default.
A modifier or country the locale lacks is never matched.

**Format** (§3, §4): UTF-8, case-sensitive. `#` lines and blank lines are comments. A
`[Desktop Entry]` group must come first; a file without one is skipped. `Key=Value`
is split at the first `=` with surrounding spaces trimmed. Key names are
`A-Za-z0-9-` plus a locale suffix. The escapes `\s \n \t \r \\` apply in string values.
Lists split on `;` with `\;` escaped, and the trailing `;` is optional. Booleans are
exactly `true` or `false`, and anything else is a parse error for that key (the key is
ignored, the entry kept). Duplicate keys in a group: the first wins (the spec forbids them; a
debug line records it). `Desktop Action` groups not named in `Actions` are ignored,
as are actions missing `Name` or `Exec` (§11).

**Caching.** Parsing every file at every open is the cost the pointer entry asks to
measure against a cache (`launcher.md:29-32`). The plan builds both and keeps the cache
only if it wins by more than the noise:

- `$XDG_CACHE_HOME/scootlaunch/index-v1` is a compact binary of the visible entries,
  their search fields and Exec argv templates, written atomically (tmp file, then
  `rename`), at most 1 MiB.
- **The key is the canonical path, inode and mtime of every directory walked**, plus the
  locale and `XDG_CURRENT_DESKTOP`. Canonical paths matter on NixOS:
  `~/.nix-profile/share/applications` is a symlink into the store, and every store
  directory has mtime 1. A profile switch therefore changes the *canonical path*, never
  the mtime, so an mtime-only key would serve a stale index forever.
- Any mismatch, a short read or a version change means a rescan. A bad cache is
  never an error.

Symlinks are followed, files and directories alike: Nix profiles are symlink trees, and
`buildEnv` links whole directories. Loops are cut by a visited `(dev, ino)` set, and
depth is capped at 8.

## Exec: from a desktop entry to an argv

Desktop Entry §7, applied after the string escapes ("the general escape rule … is
applied before the quoting rule"):

1. Split into arguments on unquoted spaces. A double-quoted argument unescapes
   `\" \` \$ \\`. Single quotes have no meaning (reserved characters must be inside double
   quotes, so an unquoted reserved character makes the entry invalid; a lenient
   mode, `exec-lenient = true`, accepts what GLib accepts, because real files break this
   rule).
2. The executable may not contain `=` (§7). If it does, the entry is invalid.
3. Field codes, outside quotes only:
   - `%f %F %u %U` are removed;
   - `%i` becomes `--icon <Icon>` as two arguments, or nothing when `Icon` is empty;
   - `%c` becomes the translated `Name`;
   - `%k` becomes the desktop file's path;
   - `%%` becomes `%`;
   - the deprecated `%d %D %n %N %v %m` are removed;
   - **any other `%x` makes the entry invalid** ("Command lines that contain a field
     code that is not listed … are invalid and must not be processed").
   
   A field code inside a quoted argument is "undefined" by the spec. Only `%%` is
   expanded there, and anything else makes the entry invalid.
4. "Field codes are expanded only once". An expansion is one argument whatever it
   contains, and is never re-split or re-scanned.
5. `Terminal=true` prefixes the configured `terminal` argv (default `["foot"]`). The
   desktop profile sets it to its own terminal.
6. `DBusActivatable=true`: call `org.freedesktop.Application.Activate`
   (`ActivateAction` for an action) on the bus name derived from the id, at the object
   path from §8 (dots to slashes, `-` to `_`, a leading `/`), with `platform_data`
   `{"activation-token": <token>}` (§8 names that key). If the call fails (no
   such service), fall back to `Exec`, as the spec still asks entries to carry `Exec`
   "for compatibility".

The parser and the expander are pure functions with one fuzz target each. Their
corpus seeds are the real desktop files of a NixOS desktop, a Fedora and a Debian
`/usr/share/applications`, plus every quoting example in §7.

## Matching and ranking

**Fuzzy match**, hand-rolled (about 200 lines; [rejected crates](shell-apps.md#rejected-alternatives)):
a case-folded subsequence match with fzf-style bonuses (at a word start, after a
separator, consecutive characters, a prefix of the field) and gap penalties. The fields
and their weights are `Name` 1.0, `GenericName` 0.6, `Keywords` 0.5, and the Exec basename
0.4. `Comment` is shown but not searched, which keeps results from turning noisy. Each entry stores a
64-bit "characters present" mask, so most entries are rejected with one AND per keystroke.
Desktop actions are their own rows ("Firefox: New Private Window"), searchable by
their names, so a keyboard user reaches them without a submenu.

**Frecency.** `$XDG_STATE_HOME/scootlaunch/history` (XDG_STATE_HOME is basedir 0.8,
default `~/.local/state`) holds text lines of `id<TAB>count<TAB>last-epoch`, at most
512 entries and 32 KiB, rewritten atomically after a launch. Malformed lines are
skipped. Weight comes from recency buckets (last 4 days ×100, 14 ×70, 31 ×50,
90 ×30, older ×10) times count. An empty query orders by frecency, then name. A typed query
orders by match score plus a frecency bonus capped at a quarter of a perfect match, so
a habit never outranks a much better match. Run mode keys the same file by `run:<argv0>`.
The dmenu mode keeps no history unless `--history NAME` is given.

**Cost.** Per keystroke: scoring into a preallocated array and a partial
selection of the visible rows (no full sort). No allocation, held by a
`count_allocations == 0` test. Target: at most 1 ms for 3,000 entries on the M2.

## Keyboard-first UX

The search field takes focus the moment the surface maps (an `overlay` layer surface with
`keyboard_interactivity: exclusive`; in scoot that "takes the keyboard as soon as the
surface maps and holds it until it unmaps", `protocols.md:1037-1040`).

| Key | Action |
|---|---|
| typing | filters (entry widget editing keys: [shell-apps.md](shell-apps.md#keyboard-and-text-entry)) |
| Up / Down, Ctrl+P / Ctrl+N, Shift+Tab / Tab | move the selection |
| Page Up / Page Down, Ctrl+Home / Ctrl+End | by page, to the ends |
| Enter | launch the selection |
| Ctrl+Enter | apps and run modes: run the typed text as a command (argv split on spaces); dmenu: print the typed text even with `--only-match` off |
| Escape, or a click outside | close (exit 1 in dmenu, as dmenu does; the call sites treat any non-zero exit as "nothing chosen", `apps-network-pick.sh:283` `\|\| exit 0`) |

Mouse: a click launches the row, and the wheel scrolls. The panel is centered on the
target output, 40 em wide with 10 rows by default, and every key is configurable.
Opening a second apps-mode instance closes the first. While open, the first instance
listens on a socket file, `$XDG_RUNTIME_DIR/scootlaunch-<WAYLAND_DISPLAY>.sock`: one fd, which
exists only while the panel is open, and is removed on close. A second invocation connects,
sends `close`, and exits. A failed connect (or a stale file) means none is open, so it opens
itself. This gives a toggle on one bind with no pid file, so there is no pid-reuse window.
dmenu instances do not listen.

A socket file rather than an abstract one, because any local user can bind an abstract name
first. `$XDG_RUNTIME_DIR` is the user's own `0700` directory, which rules that out. The listener
also checks the peer's uid with `SO_PEERCRED` and ignores any other, as scoot's own IPC does
(`crates/scoot/src/compositor/ipc/listener.rs:240-307`). scoot does it with `libc`; scootlaunch
uses rustix's safe `net::sockopt::socket_peercred` (`src/net/sockopt.rs:1692` in 1.1.5) and
keeps `forbid(unsafe_code)`.

**Which output.** The launcher should open where the user is looking. A `layer_surface` with a
null output lands on scoot's *primary* output today
(`crates/scoot/src/compositor/layer_shell.rs:228-239`), though new windows open on the pointer's
output (`protocols.md:1017-1021`). That inconsistency hits fuzzel and mako today too, neither of
which sets an output. It is the core ticket
[`layer-shell-default-output`](../../docs/backlog/core/layer-shell-default-output.md), which is
unblocked and can land before any of this. Which output counts as the one "most recently
interacted with" (the wlr-layer-shell wording) is
[Decisions needed (a)](shell-apps.md#decisions-needed). scootlaunch passes a null output and
carries no output logic of its own.

## Launching correctly

In order, on Enter (or a click):

1. **Token.** `xdg_activation_v1.get_activation_token`; `set_serial` with the
   **serial of the key press or button press that chose the entry** (scoot refuses focus
   and enter serials and accepts a key or button serial delivered to this client within
   16 events and 10 s, `crates/scoot/src/compositor/input/interaction.rs:171,193`);
   `set_surface` with the launcher's surface; `set_app_id` with the id without
   `.desktop`; then `commit`. Wait for `done`, with a 200 ms bound, after which it
   launches without a token.
2. **Environment.** Start from the launcher's own environment. Remove the
   `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID` that scoot gave the launcher itself
   (scoot strips inherited tokens only for its own spawns, `state.rs:1857-1862`, and a
   consumed token must not leak), then set the new token as `XDG_ACTIVATION_TOKEN`, and
   also as `DESKTOP_STARTUP_ID` when `DISPLAY` is set, as scoot does for X clients
   (`state.rs:1748-1752`). The working directory is `Path=`, else `$HOME`. stdin is
   `/dev/null`, stdout and stderr are inherited, and every launcher fd is close-on-exec
   (the Rust `std` default).
3. **Spawn into a scope.** Spawn the child, then on the session bus call
   `org.freedesktop.systemd1.Manager.StartTransientUnit(s name, s mode, a(sv) properties, a(sa(sv)) aux)`
   (signature checked against `org.freedesktop.systemd1(5)`) with
   `app-scootlaunch-<escaped id>-<random>.scope`, the scheme systemd documents for launchers
   (`app[-<launcher>]-<ApplicationID>-<RANDOM>.scope`), mode `fail`, and properties
   `PIDs = [pid]` and `CollectMode = inactive-or-failed`. **Verify in `launch-spawn`** which
   slice systemd's desktop-environment page asks for (`app.slice` is the expectation),
   and that the job completes before the launcher exits. The wait is bounded at
   500 ms. The child runs a few hundred microseconds in the launcher's cgroup before the
   move. GNOME and KDE accept the same window, and it is stated.
4. **Fallback** when there is no session bus or no user manager (a container, a
   non-systemd distro): double fork with `setsid`, so the app is out of the launcher's
   session and process group and is reparented. It stays in the compositor's cgroup, and the
   docs say so.
5. **Record** frecency, destroy the surface, and exit 0.

The headless `--run-id` path does steps 2 to 5 without a token (no surface, no serial).
A fresh spawn still takes focus on map, because scoot focuses every new xdg window
(`crates/scoot/src/compositor/shell.rs:40-43`).

## The dmenu contract

The paved path promised that the bar's pickers and scripts "swap daemons without
changing a call site" (`site/src/content/docs/desktop/index.md:708-718`). Every fuzzel
flag the repo's call sites pass, found by grepping `nix/modules/{apps-network-pick.sh,apps-bluetooth-pick.sh,keys-home.nix,capture-home.nix,fuzzel-theme.nix}`,
is supported with fuzzel's meaning:

| Flag | Meaning |
|---|---|
| `--dmenu` | stdin lines, the pick on stdout, exit 0; Escape exits 1 (fuzzel's own code not re-checked; the call sites only test zero) |
| `--prompt=TEXT` | prompt text |
| `--prompt-only=TEXT` | an input field with no list (the WiFi password, `apps-network-pick.sh:158`) |
| `--password` | mask the input |
| `--no-run-if-empty` | empty stdin: exit 1 without opening |
| `--only-match` | Enter with no match does nothing (without it, the typed text is printed) |
| `--list-executables-in-path` | run mode |
| `--layer=overlay` | accepted; scootlaunch is always `overlay` (`top` is hidden under fullscreen, `protocols.md:415-427`) |
| `--background-color --text-color --border-color --selection-color --selection-text-color --match-color --prompt-color` (`rrggbbaa`) | accepted as per-invocation overrides of the roles in [the table below](#stylix-and-look-integration) (`surface`, `foreground`, `focus`, `selection`, `selection-text`, `match`, `foreground`), above the user's file and the look, so `fuzzel-theme.nix`'s flags and users' scripts keep working |

Plus dmenu's own `--index` (print the 0-based index) and `--placeholder`. **Any other
flag is a loud error** (exit 2, naming the flag and the supported list), never
silently ignored. A misparsed flag that takes a value would eat the next argument. This
stays safe because the call sites in this repo are enumerated, and an eval pin
(`nix/tests.nix`) keeps them inside the supported set. The cost is that a user's own script using
another fuzzel flag fails loudly at the switch. That is chosen deliberately, and it is
[Decisions needed (c)](shell-apps.md#decisions-needed): the opt-in period is the discovery window.

Bounds: stdin is at most 1 MiB and 65,536 lines, each at most 4 KiB (longer lines are cut at a
char boundary). Non-UTF-8 input is decoded lossily. stdin is read to EOF before
the first frame (dmenu semantics), with a 5 s bound on a writer that never closes, after
which what arrived is shown.

## The agent interface

There is no compositor connection and no surface, just the same index, ranking and spawn path:

```
scootlaunch --list --json                    # every visible entry
scootlaunch --list --all --json              # NoDisplay / OnlyShowIn-hidden too, with the reason
scootlaunch --query "fire" --json            # ranked as the UI would rank it, frecency included
scootlaunch --run-id org.mozilla.firefox.desktop [--action new-private-window]
```

Each JSON entry has `id`, `name`, `generic_name`, `comment`, `keywords`, `icon` (name or
path), `exec` (the expanded argv), `terminal`, `dbus_activatable`, `actions`
(`[{id, name}]`), `path` (the desktop file) and `hidden` (null, or the reason). `--run-id` prints
`{"pid": N, "scope": "app-…scope"}` and exits 0, or exits 1 with an error object.
This is the computer-use case: an agent in a VM enumerates and launches apps
over SSH with `WAYLAND_DISPLAY` set, and sees the result through scoot's
`windows` request.

## Architecture

`crates/scootlaunch`, a binary, `#![forbid(unsafe_code)]`, single thread.

```
src/main.rs, cli.rs, config.rs (+tests)
src/index/                 pure: dirs.rs, parse.rs (+fuzz), locale.rs, exec.rs (+fuzz), cache.rs
src/rank/                  pure: fuzzy.rs, frecency.rs
src/ui/                    the surface: panel layout, list view over scootui widgets
src/spawn.rs               token, environment, scope (scoot-dbus), fallback fork
src/dmenu.rs, windows.rs, agent.rs
```

**Dependencies.**

- `scootui` features: `widgets`, `icons` + `png` (+ `svg`), `wayland`, `keyboard`.
- `scoot-dbus` for `systemd1` and `org.freedesktop.Application`.
- `scootbg-mem`.
- `wayland-client`, `wayland-protocols` (`staging`: `xdg-activation`).
- `wayland-protocols-wlr` (layer shell, foreign-toplevel management for windows mode).
- `rustix`, `serde`, `serde_json`, `toml`.

The **one candidate new external dependency is `resvg`**
(0.48.1, Apache-2.0 OR MIT, with `tiny-skia` BSD-3-Clause), because a large share of
apps ship only `scalable/apps/*.svg` icons. The lookup prefers a PNG of the right size when
the theme has one, and rasterizes SVG only when none exists. It sits behind a default-on `svg`
feature, with its size row measured in `launch-icons` for a maintainer waiver ruling
([Decisions needed](shell-apps.md#decisions-needed), item 6). Without it, the fallback is a
letter tile in the `accent` color.

**Icons**: theme lookup per the Icon Theme spec, using the configured theme (the profile passes
the look's `Adwaita`, `nix/modules/theme-look.nix`), its `Inherits` chain, then
`hicolor`, and finally `$HOME/.icons`, `$XDG_DATA_DIRS/icons` and `/usr/share/pixmaps`
(§3 order). Fixed, Scalable and Threshold directory types are handled per §4, and the
spec's pseudocode typo ("Scaled" for "Scalable") is not copied. Only the visible
rows' icons are loaded, at the device-pixel size. The cache lives only for the
process.

### Budgets

| Row | Target | fuzzel (`desktop-launcher-done.md`) |
|---|---|---|
| Closed | no process | no process |
| First frame, cache warm, 300 entries | ≤ 35 ms | 50 to 57 ms |
| First frame, cache cold | ≤ 60 ms | — |
| Open RSS, with icons | ≤ 8 MB | ~22 MB |
| Keystroke to commit, 3,000 entries | ≤ 4 ms | not measured |
| Index cold scan, 300 files | ≤ 15 ms | — |
| Stripped binary, aarch64 | ≤ 1.8 MB without `svg` (`svg` measured separately) | — |
| Closure | glibc, libgcc_s + the font | 152.4 MiB |

Measured by the method PR #451 used for fuzzel, wofi, tofi and bemenu, in the same
harness, on the M2.

## Security

Desktop files come from any package, from flatpak exports and from the user's
own directory, so they are treated as untrusted input:

- Files must be regular, opened `O_NONBLOCK`, at most 64 KiB, at most 4,096
  entries indexed, lines at most 4 KiB, at most 32 groups and 16 actions per file, and string
  values cut at 1 KiB. Control characters are dropped from anything drawn.
- Exec is never run through a shell. An argv is built ([above](#exec-from-a-desktop-entry-to-an-argv)).
  An entry whose Exec is literally `sh -c …` runs that, which is the file's own choice.
  Field-code expansion never re-splits, so a `Name` with spaces or quotes cannot inject
  arguments.
- Icon paths must be absolute, regular files, at most 8 MiB, and PNG or SVG. An SVG gets a
  node-count cap and a render-size cap, and no external references (resvg resolves none by
  default; `launch-icons` verifies it).
- dmenu stdin bounds are above. The password field is zeroized on drop
  ([shell-apps.md](shell-apps.md#keyboard-and-text-entry)), and the password is written to
  stdout only on Enter.
- History and cache files are read with the same bounds. A hostile cache gives a rescan,
  never a crash. The cache is in the user's own directory and is not a trust boundary.

## Failure modes and edge cases

| Case | Behavior |
|---|---|
| No entries at all | the panel says "No applications" (fuzzel drew an empty menu); typing still works in run mode |
| Zero outputs | exit 1 with a message |
| Another exclusive overlay surface holds the keyboard (a second launcher, a locker) | scoot gives the keyboard to the front-most (`protocols.md:1037-1040`); scootlaunch keeps drawing and waits for focus; Escape still works once it has focus |
| Session locked | scoot refuses spawns while locked and draws no layer surface; a launcher started then is not seen and exits on Escape or on its own close |
| Exec target vanished between index and launch | the spawn error is shown in the panel for 2 s, the panel stays open, and the history is not recorded |
| Activation token times out (200 ms) | launch without one; a fresh window still focuses on map |
| systemd call fails or times out (500 ms) | the app keeps running in the launcher's cgroup, the launcher exits, and a warning is logged once |
| `$XDG_STATE_HOME` or `$XDG_CACHE_HOME` unwritable | works without history or cache, warning once per run |
| Very long lists in dmenu (65,536 lines) | the virtualized list draws only the visible rows; the per-keystroke cost is bounded by the mask pre-filter |

## Stylix and look integration

scootlaunch takes its theme only through the live-looks contract
([shell-apps.md](shell-apps.md#theme-the-live-looks-contract-stylix-and-looks); PR #578 §9). At every
open it reads `<state>/look/launch.toml` (#578 §4.1), merged per key as
flags > its own `launch.toml` > the look > its defaults (#578 §4.3). It needs no reload, and it never
reads Stylix. The loader takes only the keys #578's allowlist names for scootlaunch, and
strings pass the contract's value grammar (#578 §3.4, §3.5). `terminal`, and anything else that
runs a command, is user config only.

Per element, with #578 §3.2's roles. The last column shows that the shipped looks draw what fuzzel
draws for them today:

| Element | Role | Through the `stylix` look | Shipped looks, today's fuzzel flag (`fuzzel-theme.nix:15-21`) |
|---|---|---|---|
| Panel fill | `surface` (default `background`), alpha `opacity.popups` | `base00` × Stylix `opacity.popups` | `--background-color` = `barColors.background` |
| Input and row text, prompt | `foreground` | `base05` | `--text-color`, `--prompt-color` = `barColors.foreground` |
| Placeholder, generic name, comment | `dim` | `base03` | — |
| Matched characters | `match` (default `hover`) | `base0A` | `--match-color` = `barColors.hover or barColors.accent` |
| Selected row fill | `selection` (default `accent`) | `base02` | `--selection-color` = `barColors.accent` |
| Selected row text | `selection-text` (default `accent-fg`, default `background`) | `base05` | `--selection-text-color` = `barColors.background` |
| Panel border | `focus` (default `accent`) | `base0D` | `--border-color` = `appearance.focus_ring_active_color` |
| Caret, entry focus ring | `accent` | `base0D` | — |
| Text selection in the entry | `selection` / `selection-text` | `base02` / `base05` | — |
| Corners | `shape.radius` | the contract's default | — |
| Font | `fonts.ui` at `fonts.popup-size` | `sansSerif` at `sizes.popups` | fuzzel's own font |
| Icon theme | `[icons] theme` | the contract's default | the look's `Adwaita` (`theme-look.nix`) |

Through the `stylix` look this is what Stylix's own fuzzel target draws: a `base00` panel, `base02`
selection with `base05` text, `base0A` match and a `base0D` border (#578 §9).

**Nix.** `programs.scootlaunch` has `enable`, `package`, `features` and `settings` (the user's
TOML only, into `scoot/launch.toml`). There is no `stylix.enable` and no unit.

**The profile switch.** `programs.scoot.desktop.launcher.daemon` widens to
`[ "fuzzel" "scootlaunch" ]`. With `"scootlaunch"`:

- `scoot-launcher` execs `scootlaunch "$@"` with **no color flags**. The look arrives
  through the layer, and the user's own file wins per key again, where today fuzzel's
  CLI flags beat it (`desktop/index.md:757-762`).
- The pickers' `menuPackage` default and the clipboard and capture choosers call
  `scootlaunch --dmenu` with the same flags. `fuzzel-theme.nix`'s flags are no longer passed,
  but they stay accepted, so a user's own script with them keeps working.
- `desktop.launcher.package`'s description already says it "must speak fuzzel's flags and
  `--dmenu`" (`nix/modules/launcher-home.nix:33-43`), which scootlaunch does for the
  supported set.

**Tests.**

- *Nix* (`nix/scootlaunch-tests.nix`): the rendered file equals the user's `settings`;
  fuzzel's output is byte-identical under the default; an eval pin that every flag every
  call site passes is in the supported set (it fails if a new call site adds an unsupported
  flag); `--check` over rendered files.
- *Crate*: the look-layer merge, the allowlist, the grammar, and a dangling link.
- *Goldens* per shipped look, each compared with today's fuzzel colors from the table.

## Testing and verification

- **Unit**: the desktop entry parser against every example in the spec and a fixture
  set (localization order, escapes, lists, `Hidden` masking, the id derivation,
  `OnlyShowIn` and `NotShowIn`, `TryExec`); Exec quoting and every field code,
  including the must-reject cases; matching and frecency ordering; cache keys
  (including the NixOS canonical-path case: two store dirs with mtime 1).
- **Fuzz**: the desktop entry parser, the Exec expander and the dmenu stdin splitter.
- **Headless scoot harness**: open through scoot IPC `action spawn`, type through
  `scoot msg type`, take a screenshot, press Enter, and assert the window maps and is
  focused; check that an `activate` of an already-running single-instance stub (a test client
  that keeps one window and re-activates on a token) works when chosen by mouse
  (the fuzzel regression the scoot code documents); and check the dmenu round-trip from a pipe, the
  password field drawing masked, `--no-run-if-empty`, and Escape exiting 1.
- **Scope**: under a user manager (the dev VM), `systemctl --user status` shows the
  launched app in `app-scootlaunch-…scope`, and the fallback path is exercised with no bus.
- **Goldens**: the list with a query and a selection, and the password prompt, per look and
  for a `stylix` look.
- **Ratchet** against fuzzel, tofi, bemenu and wofi.
- **M2**: open, type, launch on the real login per look, a dmenu round-trip from the bar's
  WiFi picker, and an AZERTY and a `us(intl)` dead-key query.

## Docs and packaging

- Site `/scootlaunch/`: overview with the Nix table, configure (every key), modes,
  the dmenu flag table above, the agent JSON schema, theming, and troubleshooting
  (`> **Symptom:**` "an app is missing" with `--list --all --json` showing the reason,
  "icons are letters" for the `svg` feature off or no icon theme, "opens on the
  wrong monitor"). `desktop/index.md` Launcher: the daemon choice. `agents/`:
  `--list` and `--run-id`.
- Packages: deb, rpm and Arch; `install.sh --launch`; flake outputs. CI: classify and job in
  `launch-desktop-index`, the PR that creates the crate.

## Tickets

| Slug | Serves | Blocked by |
|---|---|---|
| [`launcher`](../../docs/scootbar/backlog/launcher.md) (epic) | daily-drive, computer use | its children |
| [`launch-prototype`](../../docs/scootbar/backlog/resolved/launch-prototype-done.md): the go/no-go gate, out of the workspace | daily-drive | `scootui-keyboard-spike`, the go-ahead |
| [`launch-desktop-index`](../../docs/scootbar/backlog/launch-desktop-index.md): parser, ids, locale, Exec, cache; `--list --json` | computer use, daily-drive | the GO verdict in `launch-prototype` |
| [`launch-match-rank`](../../docs/scootbar/backlog/resolved/launch-match-rank-done.md): fuzzy, frecency; `--query --json` | both | `launch-desktop-index` |
| [`launch-ui`](../../docs/scootbar/backlog/launch-ui.md): the panel, keys, list, the look layer | daily-drive | `launch-match-rank`, `scootui-entry-and-list`, `scootui-surfaces-on-demand`, `look-format` |
| [`launch-spawn`](../../docs/scootbar/backlog/launch-spawn.md): the activation token, the environment, the systemd scope, the fallback fork, `DBusActivatable` | daily-drive | `launch-ui`, `scoot-dbus-extract` |
| [`launch-icons`](../../docs/scootbar/backlog/launch-icons.md): theme inheritance, PNG first, `resvg` behind `svg` with its size row | daily-drive | `launch-ui` |
| [`launch-dmenu`](../../docs/scootbar/backlog/launch-dmenu.md): the dmenu contract and run mode | daily-drive | `launch-spawn` |
| [`launch-agent-json`](../../docs/scootbar/backlog/launch-agent-json.md): `--run-id`, the documented schema | computer use | `launch-spawn` |
| [`launch-windows-mode`](../../docs/scootbar/backlog/launch-windows-mode.md) (low; after the switch) | daily-drive | `launch-nix` |
| [`launch-nix`](../../docs/scootbar/backlog/launch-nix.md): module, the look layer, profile switch (opt-in) | daily-drive | `launch-dmenu`, `launch-icons`, `look-format` |
| [`launch-packaging`](../../docs/scootbar/backlog/launch-packaging.md) | daily-drive | `launch-nix` |
| [`launch-default-flip`](../../docs/scootbar/backlog/launch-default-flip.md) | daily-drive | `launch-packaging`, `launch-agent-json` |
| [`layer-shell-default-output`](../../docs/backlog/core/layer-shell-default-output.md) (core) | daily-drive | none: start now |
