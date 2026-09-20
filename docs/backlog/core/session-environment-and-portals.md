---
title: "`XDG_CURRENT_DESKTOP` is set nowhere, so a desktop portal has no backend to pick"
status: "open"
area: "core"
priority: "medium"
blocked: null
---

# `XDG_CURRENT_DESKTOP` is set nowhere, so a desktop portal has no backend to pick

Filed 2026-09-19 out of the same question as
[`startup-programs-and-autostart.md`](../config/startup-programs-and-autostart.md):
what does a scoot session owe the programs running inside it?

A repo-wide grep is the finding:

```sh
git grep -rn "XDG_CURRENT_DESKTOP\|XDG_SESSION_DESKTOP\|portal" -- .
# (no output)
```

Not "set in one backend and not another" — the strings do not occur in this
project at all.

## What scoot does set, and why the omission stands out

`crates/scoot/src/compositor/mod.rs:213-238` already exports four variables
into its own environment before the loop starts, with a comment explaining
each: `WAYLAND_DISPLAY`, `SCOOT_SOCKET`, and `XCURSOR_THEME`/`XCURSOR_SIZE`.
The cursor pair is the interesting precedent — it exists so a client that
loads a cursor theme *itself* picks the same one scoot drew, i.e. the project
already accepts that a session has to tell its clients things about itself
that no protocol carries. `XDG_CURRENT_DESKTOP` is the same category, and
that block is the obvious place for it.

## What reads it

`xdg-desktop-portal` selects its backend by matching `XDG_CURRENT_DESKTOP`
against the installed backends' `UseIn=`/`portals.conf` entries. On Wayland
that is not a nicety:

- **Screen sharing.** Firefox and Chromium have no other route on Wayland —
  no portal, no screen share, which is a plain "this doesn't work" for video
  calls.
- **File chooser.** GTK and Qt apps route open/save through the portal when
  one is present; the fallback path is usually the toolkit's own dialog, so
  this one degrades rather than breaks.
- **Opening links, settings (dark mode, accent colour), inhibit, and the
  rest of the portal surface.**

The named deployment target in `README.md` is a webtop container whose main
job is running a browser, which puts this closer to the centre of what scoot
is for than its size suggests.

## The half that is not just two lines

Setting the variable in scoot's own process reaches scoot's **children**.
The portal does not start as one: it is D-Bus activated, so it inherits the
**D-Bus activation environment**, not the environment of whatever client
asked. This is why peer compositors ship a session *script* rather than only
an exported variable — niri's `resources/niri-session` (read from
`YaLTeR/niri` on `main`, 2026-09-19) calls `dbus-update-activation-environment
--all` on startup and unsets `WAYLAND_DISPLAY`, `DISPLAY`, `XDG_SESSION_TYPE`
and `XDG_CURRENT_DESKTOP` again on shutdown.

So the work splits cleanly:

1. **Export `XDG_CURRENT_DESKTOP=scoot` (and `XDG_SESSION_DESKTOP=scoot`)**
   from the existing `set_var` block. Two lines, no dependency, correct on
   its own for every child scoot starts. Worth doing with any session work
   that touches this file.
2. **Propagate to the activation environment.** `dbus-update-activation-
   environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP …`, or the
   `--all` form. Whether scoot runs this itself (a `--session` mode, which
   is what niri's flag does) or documents it as the session script's job is
   the decision, and it interacts with the autostart entry — the session
   script has to exist either way.
3. **Ship a `scoot-portals.conf`.** Backends key off the desktop name, so
   naming ourselves `scoot` means no backend claims us until a config says
   which one to use. The likely mapping is `org.freedesktop.impl.portal.
   ScreenCast/Screenshot=wlr` (scoot speaks the wlr screencopy side already —
   see `docs/protocols.md`) with `gtk` as the default for everything else.
   `portals.conf` is xdg-desktop-portal 1.17+; older versions want a
   `UseIn=` line in the backend's own `.portal` file, which is not ours to
   edit — so the flake/packaging half needs to know which it is targeting.

## Open question: `XDG_SESSION_TYPE`

Left undecided deliberately. Under `--tty` on a logind seat the session type
is already set by logind, and a compositor overwriting it is at best
redundant. Under `--nested`, and inside a container with no logind at all, it
is not set and something has to. niri's session script unsets it on exit,
which implies its session path sets it. Decide by reading what the pinned
peers actually do rather than by symmetry with `XDG_CURRENT_DESKTOP`.

## What is verified and what is not

Verified: the strings appear nowhere in the repo; scoot sets four other
variables at `mod.rs:213-238`; niri's session script does what is quoted
above.

**Not** verified: that a portal actually fails against a live scoot session.
The mechanism is well established, but this entry has not watched a file
picker fail. The cheap check is one run on the webtop target — start a
session, `busctl --user list | grep portal`, open a file dialog in a browser,
try a screen share — and it would either promote this to high with evidence
or narrow it to the parts that really break. Do that before building the
`portals.conf` half; the two-line half needs no such proof.
