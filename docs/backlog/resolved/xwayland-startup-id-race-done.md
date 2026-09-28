---
title: "XWayland: a watching X client can race a launched app to its startup id — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# XWayland: bind startup-id redemption to the spawned process — RESOLVED

RESOLVED 2026-09-27 (branch `claude/scoot-backlog-issues-3rfkfv`). While
the spawn runs (tracked, unreaped), a startup id naming its token redeems
only for a window whose X client process -- by X-Resource pid -- is the
spawn or descends from it within 8 `/proc/<pid>/stat` parent links. Once
the spawn has exited without redeeming it, the token keeps the unbound
rule it always had (review follow-up, below): **the race stays open for
launches whose process exits before its app takes focus** -- forwarders
to a running instance, forks into the background. The resolution record
is at the end; the entry as filed follows unchanged.

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

### The cost, documented (superseded: see the review follow-up below)

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

## Review follow-up: an exited spawn's token keeps the unbound rule

The completion review found the binding refused a second launch of a
single-instance X app: the spawned process forwards to the running
instance over D-Bus and exits, and the window that maps comes from the
running instance, which is not the spawn's descendant. The `gvim` fork
above is the same shape. Decision (coordinating session): while the
spawn is a live tracked child the binding holds; once it has exited, its
token falls back to the unbound rule -- any X window naming it redeems it,
once, within its 30 s. Never worse than before the binding for those
launches; the race stays closed for every launch whose process is still
running when its window maps.

### What changed

- `focus.rs`, `startup_id_bound_to`: an untracked spawn now answers
  "redeem" (was "refuse"). The tracking check runs before the
  `RefusedSpawn` cache, so a refusal cached against the spawn's pid while
  it ran cannot outlive it.
- `focus.rs`, `State::x11_focus_for_exited_spawns`, called by the `SIGCHLD`
  drain (`child_reaper.rs`) whenever it reaped a spawned child: of the
  managed X windows whose startup id (own or leader's) names a fresh token
  of a spawn no longer tracked, the lowest id (first mapped -- whom the
  unbound rule would have granted) takes focus and spends the token; a
  focused one only spends it; nothing is granted under the lock. Needed
  because the forwarder was measured to exit *after* the running
  instance's window maps (below), so a map-time decision alone would still
  refuse it. Costs one pass over the token table per reap, and a pass over
  the windows only when a fresh token outlived its spawn.
- Compositor-minted tokens with no spawn recorded are still refused for X
  windows, and a Wayland launcher's still keeps the unbound rule: neither
  carries a `SpawnedPid`, so neither is touched.

### Tests (`xwayland/tests/startup_exited.rs`, new)

- `an_exited_spawns_startup_id_is_redeemed_by_another_process` -- a real
  `State::spawn` of `sh -c 'exit 0'`, reaped as the drain does; the test
  process (the "running instance") maps a window naming the token: focused,
  token spent.
- `a_window_refused_while_its_spawn_ran_takes_focus_when_it_exits` -- the
  measured order: a `sleep 30` spawn, the window maps while it runs
  (refused, token live), the spawn is killed and reaped: focused, spent.
- `a_refusal_cached_while_the_spawn_ran_does_not_outlive_it` -- refused on
  map and again from the cache; the spawn exits and is forgotten without
  the drain's re-ask; the window's own `_NET_ACTIVE_WINDOW` then redeems.
- `a_spawn_exiting_under_the_lock_focuses_nothing` -- the re-ask grants
  nothing behind the lock and leaves the token.
- `startup_race.rs`'s `a_reaped_spawns_startup_id_is_refused` pinned the
  opposite decision and is removed; the race test
  (`a_copied_startup_id_cannot_race_the_launched_app_to_focus`, spawn
  alive) passes unchanged. The gvim-style fork is not a separate test: at
  map time it is either the first test's shape (the spawn already reaped)
  or the second's (still a zombie, refused, re-asked on reap).

**Fail first** -- uncommitted, working tree `fc3cb67` plus only the new
test file and its `mod` line:

```
soft-egl cargo nextest run -p scoot --features xwayland -E "test(/startup_exited|startup_race/)" --no-fail-fast
    FAIL ... startup_exited::an_exited_spawns_startup_id_is_redeemed_by_another_process
    assertion `left == right` failed: an exited spawn's startup id was refused
      left: Some(WindowId(1))
     right: Some(WindowId(2))
    FAIL ... startup_exited::a_window_refused_while_its_spawn_ran_takes_focus_when_it_exits
    assertion `left == right` failed: the window was not re-asked for when its spawn exited
      left: Some(WindowId(1))
     right: Some(WindowId(2))
    FAIL [  20.446s] ... startup_exited::a_refusal_cached_while_the_spawn_ran_does_not_outlive_it
    PASS ... startup_exited::a_spawn_exiting_under_the_lock_focuses_nothing
     Summary [  20.450s] 8 tests run: 5 passed, 3 failed, 2028 skipped
```

The lock test passes vacuously before the fix (nothing was re-asked); it
and the cache test were each mutation-checked against the fix: dropping
the `is_locked()` guard fails the lock test, and consulting the
`RefusedSpawn` cache before the tracking check fails the cache test.

### Measured: GTK `mousepad`, second launch

`scoot --headless --xwayland` under `dbus-run-session`, `GDK_BACKEND=x11`,
`mousepad --opening-mode=window` spawned, then `foot`, then `mousepad`
again (`RUST_LOG=scoot=debug`). Built from `fc3cb67` plus a temporary
debug line (not committed) printing the window's startup ids -- the gate
as before this follow-up:

```
00:29:13.452884Z INFO  spawned command=["mousepad", "--opening-mode=window"]
00:29:13.557199Z DEBUG X11 window created id=4194363
00:29:13.559560Z DEBUG X11 window mapped id=WindowId(3) xid=4194363 focus=false
00:29:13.567693Z DEBUG reaped a spawned child pid=30506 status=0
```

Re-run on the fix (uncommitted tree: this follow-up, probe line removed):
the same shape -- mapped at `00:40:19.180392`, `focus=false`; forwarder
reaped at `00:40:19.188479` -- and still unfocused, for the reason below.

The forwarder is reaped 8 ms *after* the window maps -- why the re-ask on
exit exists. But for GTK 3 `mousepad` the new window names no startup id
scoot can see, before this branch or after it: a temporary probe logged
`own=None leader=None group=Some(4194305)` for both launches' windows, and
`xprop` showed the leader `0x400001` still carrying the *first* launch's id
after the second (the toplevel carries none). scoot's XWM never reported
the leader as a window (no `X11 window created id=4194305`), consistent
with Smithay skipping CreateNotify for non-`InputOutput` windows (GTK 3's
leader is believed to be `InputOnly`; not checked directly). So mousepad's
second window opens unfocused on `main` too, and after this change; the
first launch focuses by process. The fallback matters for toolkits that
do put the forwarded id on a window scoot sees; none was available here
to measure.

### What remains open

- **The race, for launches whose process exits before its app takes
  focus**: a watching X client can copy the id and win that launch's
  focus, once, within the token's 30 s -- exactly as before the binding.
- A launched process that keeps running while a process it did not start
  maps the window (a client that waits on a server) is still refused;
  unmeasured whether any X app does this with a startup id.
- The client-leader startup-id path does not reach GTK 3's leader (above).

