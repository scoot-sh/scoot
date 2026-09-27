---
title: "XWayland: a watching X client can race a launched app to its startup id — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# XWayland: bind startup-id redemption to the spawned process — RESOLVED

RESOLVED 2026-09-27 (branch `claude/scoot-backlog-issues-3rfkfv`). A
startup id naming a spawn token now redeems only for a window whose X
client process -- by X-Resource pid -- is the spawn or descends from it
within 8 `/proc/<pid>/stat` parent links, while the spawn is still tracked
(unreaped). The resolution record is at the end; the entry as filed
follows unchanged.

---

Filed 2026-09-25 from the PR #244 (XWayland Phases 2+3) completion review.
Serves computer use and daily use alike: the focus gate is a security
property, and this is its one known window.

## The gap

The X focus gate (`compositor/xwayland/focus.rs`) lets a mapping X window
take focus when its `_NET_STARTUP_ID` -- or its client leader's, the
`WM_HINTS` window group, same X client only, where GTK sets it -- names a live activation
token; `State::spawn` hands every child its token as `DESKTOP_STARTUP_ID`
while XWayland is live. But a startup id is a property readable by every X
client, and a toolkit sets it on its leader at startup, before its first
window maps. A background X client watching the root can read it, copy it
onto a window of its own, and map *before the app does*: it redeems the token and takes
focus from a Wayland window once. The real app arrives second; the token
is single-use. The window is from the moment the app sets its startup id
until its own first window maps (which spends the token, whichever gate
rule grants focus -- review round 1 closed the case where a rule-1 or
rule-3 grant left it live for later), bounded by the token's 30 s.

The redemption does not check the redeeming window's process. The existing
tests (`a_spawn_tokens_startup_id_lets_an_x_window_take_focus_once`,
`a_startup_id_on_the_client_leader_is_redeemed`) show it: the redeeming X
client is the test process, not a spawned child.

(A same-uid process can also read any child's token from
`/proc/<pid>/environ`. That is the project's documented same-uid trust
boundary and applies to every activation token, X or Wayland; out of scope
here.)

## Proposed tightening

When the token carries `SpawnedPid(p)` (every token scoot mints for a spawn
while XWayland is live), accept a startup-id redemption only if the window's
X-Resource client pid is `p` or a descendant of it -- a bounded walk up
`/proc/<pid>/stat` ppids (a few levels), so wrapper scripts (`sh -c`,
launcher shims, and -- unverified -- `flatpak run`) keep working. A token without `SpawnedPid` (a
Wayland launcher's, minted from a real click) keeps today's rule, or is
bound to nothing and refused for X -- decide and document.

Cost: one `/proc` walk per startup-id redemption (a map, not a hot path).
Test: a second X client (another process -- `xeyes`, or a helper binary)
copying the id and mapping first must be refused, and the real child still
focused.

## What done looks like

The race test fails first against today's gate and passes after; the
`focus.rs` and `protocols.md` "one known window" notes are removed; full
gate green.

## Resolution

### What changed

- `compositor/xwayland/ancestry.rs` (new): `descends_from(pid, ancestor)`,
  a walk up `/proc/<pid>/stat` parent pids bounded by
  `MAX_ANCESTRY_DEPTH = 8` links, heap-free (path formatted into a stack
  buffer, only a 256-byte head of each `stat` read into another). The
  parent pid is read after the *last* `)` -- `comm` is process-chosen and
  may contain `) S 1` -- and must be followed by its separator, so a head
  cut short mid-number is no answer rather than a shorter pid. Any read or
  parse failure answers "not a descendant".
- `compositor/xwayland/focus.rs`: the startup-id half of rule 2 now asks
  `startup_id_bound_to` before spending the token:
  - a token carrying `SpawnedPid(p)` redeems only if `p` is still in
    `spawned_children` (unreaped, so the pid is not reused) and the
    window's X-Resource client pid descends from `p`;
  - a **Wayland client's token** (`client_id` set: a launcher's, which
    passed `activation.rs`'s serial gate) keeps the unbound rule;
  - a token with neither (scoot-minted for a spawn while XWayland was not
    live; never handed to an X toolkit) is refused for X windows.
  A refused window leaves the token live for the process it belongs to.
  An unknown client pid (X-Resource query failed or refused; Smithay
  reports that as pid 0, which `client_pid` maps to `None`) fails closed.
  The "one known window" notes in `focus.rs` and `docs/protocols.md` are
  gone, replaced by the launcher-token residual below.

### The decision on tokens without `SpawnedPid`

A Wayland launcher's token keeps the unbound rule, and the race stays open
for it. scoot never learns which process a launcher started -- a launcher
typically exits straight after launching, so the app is reparented away
from it and no process tree ties them -- and GLib-based launchers hand the
token over as `DESKTOP_STARTUP_ID` too, so refusing these would open every
X app a GTK launcher or file manager starts behind the window the user
launched it from. The residual is documented in `focus.rs`,
`docs/protocols.md` and the changelog. A compositor-minted token with no
spawn recorded is refused: no X toolkit was ever handed it.

### The cost, documented

An X app that forks into the background and lets the process scoot
started exit (`gvim` without `-f`, per its documentation; not measured
here) is reparented away from the spawn, so its startup id no longer
redeems: it maps unfocused while another window has focus. Documented in
`focus.rs`, `docs/protocols.md` and `CHANGELOG.md`.

### X-Resource

Already in use by the gate (rule 2's process path and rule 3):
Smithay's `X11Surface::get_client_pid` sends `XResQueryClientIds` with
`LOCAL_CLIENT_PID` through x11rb's `res` feature (fork `7e18b66`,
`src/xwayland/xwm/surface.rs:2013`), and returns `Ok(0)` when the request
cannot be sent (extension absent) or `Err` when the reply fails or names
no pid. `client_pid` treats both as unknown. XWayland 24.1.13 answers it:
the live race test's launched app redeems by it.

### Tests

- `xwayland/tests/startup_race.rs` (new):
  - `a_copied_startup_id_cannot_race_the_launched_app_to_focus` -- the
    launched app is real: this test binary re-run through `State::spawn`
    and an `sh -c` wrapper (not `exec`, so the app is the token's spawn's
    child), as the `#[ignore]`d helper `launched_app` (a no-op unless the
    wrapper sets its env var). It creates a client leader carrying
    `$DESKTOP_STARTUP_ID`, as GTK does, then waits for a go property. The
    test process -- another process -- reads the id off the leader over X,
    maps a window with it and sends `_NET_ACTIVE_WINDOW`: both refused,
    token still live. Then the app maps in its leader's group and takes
    focus from the Wayland window; the token is spent.
  - `a_reaped_spawns_startup_id_is_refused`,
    `an_unbound_compositor_token_is_refused_for_x`,
    `a_wayland_launchers_token_keeps_the_unbound_rule` pin the other three
    branches.
- `xwayland/ancestry/tests.rs` (new): hostile `comm` fields, cut-short
  heads, self/parent/child/missing pids, and a real 11-process `sh` chain
  proving the walk stops at 8 links.
- The existing startup-id tests now bind their token the way `State::spawn`
  does (`Live::launch_token`: `SpawnedPid` of the test's parent, tracked as
  a spawned child), so the test process redeems as a wrapped descendant --
  and still not by process, which keeps
  `a_startup_id_on_the_client_leader_is_redeemed` about the leader.

### Evidence

All on the Claude Code web container (root, no GPU; `soft-egl`), PATH
extended with Nix store `xclock`, `xeyes`, `procps` and `xwayland-24.1.13`,
`SCOOT_REQUIRE_XWAYLAND=1`.

**Fail first** -- uncommitted, working tree `17056ae` plus only
`tests/startup_race.rs` and its `mod` line (gate unchanged):

```
cargo nextest run -p scoot --features xwayland -E 'test(=compositor::xwayland::tests::startup_race::a_copied_startup_id_cannot_race_the_launched_app_to_focus)'
    thread '…a_copied_startup_id_cannot_race_the_launched_app_to_focus' (5937) panicked at crates/scoot/src/compositor/xwayland/tests/startup_race.rs:93:5:
    assertion `left == right` failed: a copied startup id raced the launched app to focus
      left: Some(WindowId(2))
     right: Some(WindowId(1))
     Summary [   0.255s] 1 test run: 0 passed, 1 failed, 1919 skipped
```

(`WindowId(2)` is the racer's window, `WindowId(1)` the Wayland window.)

**Mutation** -- the fix with the walk disabled (`pid == spawned &&
descends_from(…)`, exact pid only): six focus/race tests fail, among them
the race test (the launched app, a wrapper's child, can no longer redeem)
and `a_startup_id_on_the_client_leader_is_redeemed`; restored after.

**Cost** -- a throwaway `#[ignore]`d probe (not committed) timing
`descends_from` 20 000 times in a release test build
(`cargo nextest run -p scoot --features xwayland --cargo-profile release
--run-ignored only`), working tree as of the fix plus the per-window
refusal cache:

```
PROBE depth_to_init=7 walk_to_init=28.219µs one_link=3.698µs
```

So a refused walk to init is ~28 µs and a one-link wrapper ~4 µs, paid
once per startup-id redemption (a map or a `_NET_ACTIVE_WINDOW`), and a
refusal once per window and spawn (the `RefusedSpawn` cache on the
surface): a client spamming `_NET_ACTIVE_WINDOW` with a copied id pays it
once. Before the change the redemption did no walk (0 µs); nothing on a
per-event or per-frame path changed.

**Pass after** -- see the PR description for the full gate's raw tails
against the committed SHA.
