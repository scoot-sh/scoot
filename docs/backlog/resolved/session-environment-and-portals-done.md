---
title: "`XDG_CURRENT_DESKTOP` is set nowhere, so a desktop portal has no backend to pick — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `XDG_CURRENT_DESKTOP` is set nowhere, so a desktop portal has no backend to pick — DONE

Filed 2026-09-19 out of the same question as
[`startup-programs-and-autostart.md`](startup-programs-and-autostart-done.md):
what does a scoot session owe the programs running inside it. A repo-wide
grep was the finding: the strings `XDG_CURRENT_DESKTOP`,
`XDG_SESSION_DESKTOP` and `portal` occurred nowhere in `crates/ vm/
scripts/` (re-verified empty on `main` at `6a5bb07` before this work), while
`compositor/mod.rs` already exported `WAYLAND_DISPLAY`, `SCOOT_SOCKET`,
`XCURSOR_THEME` and `XCURSOR_SIZE` for exactly this category of reason.

## Verify-first: what the probe actually showed

The ticket asked for a live portal failure before building the
`portals.conf` half. Run on the dev VM (`ssh -p 2222 dev@localhost`):

```sh
busctl --user list | grep -i portal   # no output -- not even an activatable name
for b in firefox chromium google-chrome xdg-desktop-portal; do command -v $b; done  # none found
```

The user bus carries **no portal stack at all** -- no
`org.freedesktop.portal.Desktop`, no activatable backend names, no browser,
no `xdg-desktop-portal` binary -- and `dbus-update-activation-environment`
and `systemctl` exist but there is nothing portal-shaped to observe. So a
file picker / screen share could not be watched to fail against a live
scoot session in this environment, and no such watch is claimed here.

What that settles, per the ticket's own staging: the two-line env half
needs no such proof and landed regardless; the `portals.conf` half is gated
not on a live failure but on its own stated precondition -- *answer whether
a backend exists that speaks `ext-image-copy-capture-v1` before writing the
file*. That question turned out to have a different answer than the ticket
recorded (see below), verified in upstream source rather than live.

## The ticket premise that went stale: a backend speaks ext-capture now

The ticket proved `wlr` was not the answer *at the time*: only
`xdg-desktop-portal-wlr` was on the table and it needed
`zwlr_screencopy_manager_v1`, which scoot deliberately does not implement.
Two upstream facts since then change the verdict:

- **xdg-desktop-portal-wlr v0.8.0** (21 Oct, commit `0ab4f6f`, verified on
  the project's own releases page) ships "`ext_image_copy_capture_v1:
  Initial   implementation`" (Kenny Levinsen), with an ext-path follow-up in
  0.8.2 ("check for capture source manager in
  `ext_register_session_cb`"). So `wlr` binds against an ext-only
  compositor through the ext protocol -- provided the backend is >= 0.8.0.
  (Avoid 0.8.3: its own release notes warn it can stall screen recording;
  0.8.4+ is the boring choice.)
- **xdpw's Screenshot portal shells out to `grim`** (verified in
  `src/screenshot/screenshot.c` at `master`: `execvp("grim", ...)`), and
  grim (>= 1.5.0) speaks *only* `ext-image-copy-capture-v1` -- which scoot
  implements. So the Screenshot mapping needs no wlr-screencopy either,
  only an installed `grim`.

The GNOME backend is deliberately *not* named: niri earns it by
implementing `org.gnome.Mutter.ScreenCast`, which scoot does not. The
`default=gtk` fallback is the shape xdpw's own README documents
(`default=gtk`, `Screenshot=wlr`, `ScreenCast=wlr`) and both niri and sway
recommend for the non-capture portals.

## What landed

**Env exports** (`compositor/session_env.rs`, new; applied in
`compositor/mod.rs`'s `set_var` block and per child in `State::spawn`):

- `XDG_CURRENT_DESKTOP=scoot`, unconditional.
- `XDG_SESSION_TYPE=wayland`, `XDG_SESSION_DESKTOP=scoot`, only when unset
  or empty (an empty string counts as unset -- no reader treats it as
  meaningful, and Qt checks emptiness).
- One decision site: the pure `session_env::resolve`, unit-tested without
  touching the process-global environment; `run` settles the process env,
  `spawn` re-resolves per child (idempotent) and sets the three explicitly,
  the way `WAYLAND_DISPLAY` already is.

**`resources/scoot-portals.conf`** (new): `[preferred]` with
`default=gtk`, `Screenshot=wlr`, `ScreenCast=wlr`, with the version floors
(xdg-desktop-portal 1.17+, xdg-desktop-portal-wlr 0.8.0+, installed `grim`)
and the install paths (`~/.config`, `/etc`, `/usr/share` under
`xdg-desktop-portal/scoot-portals.conf`) in comments. Wiring the install
step into the flake is *not* done -- the flake's `src` fileset is scoped to
`Cargo.toml`/`Cargo.lock`/`crates/` by an explicit verified-coverage claim,
and an install step no portal stack on any test machine can exercise would
be unverifiable wiring. Until the packaging half lands (natural home: the
flake-consumer ticket), the copy-over is manual and documented in
`docs/configuration.md`.

**Docs** (`docs/configuration.md`): the env-exports paragraph now names all
three variables with their semantics, plus a "Portals and the D-Bus
activation environment" subsection with the session-script commands
(systemd and non-systemd shapes), the conf file's install paths, and the
version floors.

**Tests**: four pure `resolve` unit tests (unconditional overwrite incl. a
host `gnome`/`sway` value, keep-where-set for all three logind-shaped
values, fill-where-unset-or-empty, borrow-not-copy pinning the
no-allocation claim); one live-child harness test
(`activation/tests/session_env.rs`, `sh`-probe-to-file shape from
`spawn.rs`) asserting `XDG_CURRENT_DESKTOP=scoot` exactly plus non-empty
session type/desktop; a smoke-test section asserting the same three ride
along in a live `foot`'s `/proc/$pid/environ`.

## The three decisions, with reasoning

1. **`XDG_CURRENT_DESKTOP`: set unconditionally, not set-if-unset.** Who
   reads it is a scoot child talking to scoot's compositor, so it must name
   the compositor the child actually talks to. Under `--nested` inside
   another compositor (or a session that already exports one), inheriting
   would key portal lookup to the wrong compositor -- scoot's own
   `scoot-portals.conf` would never be consulted, and a host-keyed backend
   could address the host session instead of this one. A colon-joined
   `scoot:<host>` list was considered and rejected for the same reason: the
   fallback names would resolve capture portals against compositors the
   client is not talking to. niri sets its own name unconditionally; scoot
   does the same, for the stated reader-based reason rather than by copying.
2. **`XDG_SESSION_TYPE` / `XDG_SESSION_DESKTOP`: fill the vacuum, never
   overwrite.** Both are logind-owned on a seat (`pam_systemd` sets the
   first, reads the second as its `desktop=` input); overwriting logind's
   values is at best redundant, which is exactly the ticket's open question.
   Where no logind exists -- `--nested`, a container with no seat -- nothing
   sets them and Qt/xdg-autostart readers need them, so scoot fills `wayland`
   / `scoot`. This differs from niri (unconditional `wayland`) deliberately,
   and answers the ticket's "say which cases step 1 is for": the no-logind
   cases only; on a logind seat the owner's values stand -- including under
   `--nested`, where the logind session really is the host's.
3. **Activation propagation: the session script's job, documented; scoot
   does not spawn bus tools.** niri does both (compositor import under
   `--session` plus the session script's `--all`), but scoot has no
   `--session` concept and ships no session script, and a compositor-side
   spawn buys a hang (a wedged bus stalls startup with no timeout), a zombie
   (the reaper tracks only `spawn` pids, never `waitpid(-1)`), and
   per-system `--systemd`-or-not logic -- all for a line the session author
   writes once in the right context. This does not contradict
   `startup-programs-and-autostart.md`: that entry already names the D-Bus
   activation environment as the session script's mechanism, and it stays
   open (supervision/autostart config is explicitly out of scope here).
   (Update 2026-09-20: the autostart half has since resolved — see
   `startup-programs-and-autostart-done.md`; supervision remains out of
   scope.)

Also decided, from the bug-bash: **nothing is unset on session end.** The
process environment dies with the process, and scoot never writes the
activation environment, so there is nothing to retract -- niri unsets from
the systemd manager precisely because it wrote there. A surviving child
keeping a stale `WAYLAND_DISPLAY` at a dead socket is pre-existing and the
same for these variables, not new harm.

## Bug-bash (edge cases, explicitly)

- Nested-under-another-compositor inheritance: covered by decision 1;
  pinned by the `Some("gnome")`/`Some("sway")` unit cases.
- Logind seat where vars already set: covered by decision 2; pinned by the
  keep-where-set unit cases (`x11`, `tty` for type; third-party desktops
  for desktop). No clobbering of logind-owned state.
- Child that unsets/inherits: a child's own copy affects only itself;
  `Command` inherits by default and `spawn` sets explicitly on top, so the
  contract holds whichever the child does.
- Empty-string values: treated as unset (filled); pinned by the
  `Some("")` unit cases.
- Session end: nothing to unset (above); recorded, not deferred -- there is
  no user-facing harm in scope (no activation writes to retract).
- `portals.conf` claiming a backend that binds nothing: the gated harm.
  Gated by version floor, not by omission: ScreenCast needs xdpw >= 0.8.0
  (older needs the deliberately-unimplemented wlr-screencopy global),
  Screenshot needs `grim` installed, and both floors are stated in the
  file's own comments and in `docs/configuration.md`. A stale-backend
  failure surfaces as a refused portal request, never as data loss or a
  wedged session.

## Benchmark

n/a, by the ticket's own terms: process-startup `set_var` on a cold path
plus three `var` reads and a branchless pure function per `spawn` (itself a
`fork`+`exec`, milliseconds). No hot or per-event/per-frame path is
touched; no numbers taken.

## Left out, with why

- **Live portal proof** (file dialog / screen share against a real portal
  stack): impossible on the dev VM -- no portal services, no browser (probe
  above). The env half is proven live (`/proc/$pid/environ` of a real
  spawned child, harness + smoke); the conf half rests on upstream-source
  verification (release notes + `screenshot.c`) with the floors written
  down, not on a live run. A machine with xdg-desktop-portal-wlr >= 0.8.0
  and a browser can close this gap by installing the file and screen
  sharing; that run is not claimed here.
- **Flake install wiring** for the conf file: filed as the packaging
  remainder (above).
- **Supervision/autostart config, wlr-screencopy, multi-output**: explicitly
  out of scope per the ticket; untouched.

## Evidence

Implementation commit: `ce69ffe` on branch
`backlog/session-environment-portals` (all evidence below captured against
that tree; the `nextest`/`clippy`/`fmt`/`smoke` runs below ran after the
final source edit, so the key is unmodified).

- `cargo nextest run --workspace`: 1162 passed, 4 skipped, 0 failed (dev VM).
- `cargo clippy -p scoot --all-targets -- -D warnings`: clean (dev VM).
- `cargo fmt --check -p scoot`: clean (dev VM).
- `scripts/smoke-test.sh` incl. the new session-environment section: green
  end to end (dev VM, `--headless`, binary `/var/cargo-target/debug/scoot`
  built from this branch).
- Fail-first: with the three `child.env` lines in `State::spawn` removed,
  `a_spawned_child_sees_the_session_environment` fails
  (`left: "<unset>", right: "scoot"`); restored, it passes.
- Live proof, raw: a real headless session
  (`scoot --headless -- sleep 57`) started on the dev VM, child's
  `/proc/<pid>/environ` read directly:

  ```text
  SCOOT_SOCKET=/tmp/scoot-envproof2/scoot.sock
  WAYLAND_DISPLAY=wayland-1
  XDG_ACTIVATION_TOKEN=3eQyyCxEpboqsDPZekVyC59F9wSGHAa2
  XDG_CURRENT_DESKTOP=scoot
  XDG_SESSION_DESKTOP=scoot
  XDG_SESSION_TYPE=tty
  ```

  Note `XDG_SESSION_TYPE=tty`: the ssh login session's value, kept rather
  than clobbered -- decision 2 working live -- while the unset
  `XDG_SESSION_DESKTOP` was filled with `scoot` and `XDG_CURRENT_DESKTOP`
  set unconditionally. (The `sleep 57` child was `kill`ed and the session
  `pkill -x scoot`ed afterwards; no scoot processes left.)
- Portal probe: `busctl --user list | grep -i portal` empty (no
  `org.freedesktop.portal.Desktop`, not even activatable);
  `firefox`/`chromium`/`xdg-desktop-portal` all absent from `PATH` on the
  dev VM (NixOS 26.11, user bus over `ssh -p 2222 dev@localhost`).
- Upstream sources: xdg-desktop-portal-wlr releases page (v0.8.0,
  `0ab4f6f`, "ext_image_copy_capture_v1: Initial implementation"),
  `src/screenshot/screenshot.c` at `master` (`execvp("grim", ...)`),
  xdpw README (activation command + `[preferred]` shape), niri wiki
  Important-Software (gtk fallback + gnome-for-screencast via
  Mutter.ScreenCast, which scoot does not implement).
