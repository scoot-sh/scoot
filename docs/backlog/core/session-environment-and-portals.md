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
git grep -rn "XDG_CURRENT_DESKTOP\|XDG_SESSION_DESKTOP\|portal" -- crates/ vm/ scripts/
# (no output)
```

Not "set in one backend and not another" — the strings do not occur in this
project's code at all. (Scoped to `crates/ vm/ scripts/` so it keeps
reproducing: run against the whole tree it now matches this entry. Against
`main` as of `fb7ff53`, the unscoped form was also empty.)

## What scoot does set, and why the omission stands out

`crates/scoot/src/compositor/mod.rs` already exports four variables into its
own environment before the loop starts (the `set_var` block is 218-237,
under a Safety comment from 213), each with a comment explaining it:
`WAYLAND_DISPLAY`, `SCOOT_SOCKET`, `XCURSOR_THEME` and `XCURSOR_SIZE`.
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
- **File chooser.** GTK routes open/save through the portal when sandboxed
  or when `GTK_USE_PORTAL=1`; Qt when sandboxed or under
  `QT_QPA_PLATFORMTHEME=xdgdesktopportal`. Narrower than "always", and the
  fallback is the toolkit's own dialog — so this one degrades rather than
  breaks.
- **Opening links, settings (dark mode, accent colour), inhibit, and the
  rest of the portal surface.**

The named deployment target in `README.md` is a webtop container whose main
job is running a browser, which puts this closer to the centre of what scoot
is for than its size suggests.

## The half that is not just two lines

Setting the variable in scoot's own process reaches scoot's **children**.
The portal does not start as one: it is D-Bus activated, so it inherits the
**D-Bus activation environment**, not the environment of whatever client
asked. This is why peer compositors do both halves — set the variable *and* push it
outward. niri (read from `YaLTeR/niri` on `main`, 2026-09-19) sets
`XDG_CURRENT_DESKTOP=niri` and `XDG_SESSION_TYPE=wayland` in-process under
`--session` (`src/main.rs:94-96`), then calls `systemctl --user
import-environment` and `dbus-update-activation-environment` for those
variables (`main.rs:226` → `import_environment()`, `:285-316`). Its
`resources/niri-session` script calls `dbus-update-activation-environment
--all` in both its systemd and dinit branches, and on shutdown unsets
`WAYLAND_DISPLAY`, `DISPLAY`, `XDG_SESSION_TYPE`, `XDG_CURRENT_DESKTOP` and
`NIRI_SOCKET` *from the systemd user manager's environment*.

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
   which one to use. The file shape is settled — Hyprland's is literally a
   `[preferred]` section with `default=hyprland;gtk` — and `gtk` is the
   obvious default for the non-capture portals. `portals.conf` is
   xdg-desktop-portal 1.17+; older versions want a `UseIn=` line in the
   backend's own `.portal` file, which is not ours to edit, so the
   flake/packaging half needs to know which it is targeting.

   **Which backend handles ScreenCast/Screenshot is genuinely open, and
   `wlr` is not the answer today.** `xdg-desktop-portal-wlr` requires
   `zwlr_screencopy_manager_v1`, and `docs/protocols.md:52-57` lists
   `wlr-screencopy-v1` under *"Not implemented"* as a **deliberate**
   decision — scoot implements `ext-image-copy-capture-v1` +
   `ext-image-capture-source-v1` instead, because the clients that motivated
   capture (grim 1.5.0, quickshell 0.3.1) speak `ext-`. Pointing a config at
   `wlr` would produce the worst outcome available: an installed backend
   that binds no global and fails every request, with a config file and a
   backlog entry both asserting it should work. So the real question is
   whether a backend exists that speaks `ext-image-copy-capture-v1`, or
   whether this is the concrete reason to revisit that deliberate decision.
   Answer that before writing the file.

## Open question: `XDG_SESSION_TYPE`

Under `--tty` on a logind seat the session type is already set by logind, and
a compositor overwriting it is at best redundant. Under `--nested`, and
inside a container with no logind at all, it is not set and something has to.

niri answers this one unconditionally: `src/main.rs:96` sets
`XDG_SESSION_TYPE=wayland` under `--session`, commented *"for xdg-autostart
and Qt apps"*. So the precedent is "set it anyway". The reason to still
think rather than copy is that scoot's `--nested` and webtop cases are
exactly where a wrong value would mislead, and overwriting logind's is the
one case with a real owner.

Note also that niri does **not** set `XDG_SESSION_DESKTOP`, which step 1
above proposes — and there is a good reason, which strengthens rather than
weakens the case for thinking about it. `XDG_SESSION_DESKTOP` is
**logind-owned**: `pam_systemd(8)` documents it as a PAM environment
variable read at session registration (preferred over the module's own
`desktop=` argument). On a logind seat it is already someone else's to set,
which is exactly the `XDG_SESSION_TYPE` situation above. Where it is *not*
already set — `--nested`, a container with no logind — the same argument for
setting it applies. Say which of those cases step 1 is for.

## What is verified and what is not

Verified: the strings appear nowhere in the project's code; scoot sets four
other variables at `mod.rs:218-237`; niri does what is quoted above (its
session script read from the raw file, its `--session` behavior read from
`src/main.rs`); `portals.conf` is 1.17+; `xdg-desktop-portal-wlr` needs
`zwlr_screencopy_manager_v1`, which `docs/protocols.md` says scoot
deliberately does not implement.

**Not** verified: that a portal actually fails against a live scoot session.
The mechanism is well established, but this entry has not watched a file
picker fail. The cheap check is one run on the webtop target — start a
session, `busctl --user list | grep portal`, open a file dialog in a browser,
try a screen share — and it would either promote this to high with evidence
or narrow it to the parts that really break. Do that before building the
`portals.conf` half; the two-line half needs no such proof.
