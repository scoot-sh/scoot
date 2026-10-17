---
title: "nested binds the host's own wayland socket, talks to itself, and deletes the host socket on exit"
status: "open"
area: "core"
priority: "high"
blocked: null
---

# nested binds the host's own wayland socket, talks to itself, and deletes the host socket on exit

Filed 2026-10-17 from the webtop field test (T-webtop). Serves **daily-drive**:
`scoot --nested` is the documented webtop route (`site/src/content/docs/agents/webtop.md`)
and the "try it in a window" route; against a host on `wayland-1` it never starts AND
leaves the host deaf to new clients.

## The gap

`scoot --nested` with `WAYLAND_DISPLAY=wayland-1` (pixelflux in
`lscr.io/linuxserver/webtop:debian-xfce` + `PIXELFLUX_WAYLAND=true`,
`XDG_RUNTIME_DIR=/config/.XDG`) fails its startup registry roundtrip every time:

```
scoot: the host compositor did not answer the startup registry request within 10s; stopping the nested session
```

strace on the failing process shows why — `State::new` binds scoot's OWN
listening socket first, and takes the host's name:

```
bind(9, {sun_path="/config/.XDG/wayland-1"}) = 0
connect(12, {sun_path="/config/.XDG/wayland-1"}) = 0
sendto(12, "<get_registry+sync>", 24) = 24
recvmsg(12) = ECONNRESET
```

Mechanism, verified against the pinned sources:
1. `WaylandListener::bind_auto` (`wayland_accept.rs:88`) tries `wayland-1`
   through `wayland-32` (`ListeningSocket::bind_auto("wayland", 1..33)`).
   `wayland-0` is deliberately skipped — but nothing skips the name the host
   is actually on.
2. wayland-server 0.31.14 `bind_absolute` removes an existing socket file
   once it holds the `.lock` (`socket.rs`: "one exist, remove it"). The lock
   is an `flock`; pixelflux does not hold an exclusive `flock` on
   `wayland-1.lock`, so scoot unlinks the LIVE host socket and binds the name.
3. `nested::init` then connects to `$WAYLAND_DISPLAY` — i.e. to scoot itself.
   `mod.rs:505-514` orders the connect before the `set_var`, which is
   correct and beside the point: the clobber happens one layer down, at bind
   time, before any of that runs.
4. scoot's own display is not dispatching yet (nested init runs pre-loop),
   so the roundtrip never completes: 10 s timeout, exit 1, and `Drop`
   removes scoot's socket file — the host's `wayland-1` is gone for good.
   pixelflux keeps running (established clients like Xwayland survive; the
   web UI keeps serving), but no new Wayland client can ever connect.

Isolation proof (same settled boot, same socket, seconds apart):
- `wayland-info` x3: full global list, socket survives every time.
- a hand-rolled probe (connect + get_registry + sync, bind nothing): 26
  globals + done, socket survives.
- `scoot --nested -- /bin/true`: silence, reset, socket deleted — every time,
  across 6+ boots, at-boot and settled alike.

So pixelflux answers every well-behaved client; only scoot clobbers the path
it then reads from. Plain `wayland-info` control included because the first
suspicion was a deaf host.

## What to do

Minimum: under `--nested`, never bind the host's display name. The name is
known at startup (the pre-`set_var` `WAYLAND_DISPLAY`), so filter it out of
the `bind_auto` range — or bind the session socket only after the host
roundtrip succeeds, on a name verified free by more than the lock file.
Defense in depth, pick at least one:
- after connecting, compare the connected socket against the listening
  socket (fstat dev/ino) and refuse loudly with "connected to itself" instead
  of timing out;
- liveness-probe an existing socket file (connect test) before unlinking it,
  so a lock-file scheme a host does not share cannot delete a live server.

Pin the edge cases: host on `wayland-0` (currently safe by the skip — keep
it safe); host on `wayland-5` with a squatter on 1..4 (must land past them,
never on them); TWO nested scoots in one runtime dir (must not take each
other's names — the lock file covers this today, keep it covered); the
refusal message must name both paths when it fires.

Regression test shape: a fake host compositor holding `wayland-1` WITHOUT an
flock on the lock (the pixelflux shape), then `scoot --nested` must land on
another name and complete the registry roundtrip. The current code fails it
by deleting the host socket.

## Not in this ticket

- The foot `initial-color-theme` refusal and the Alpine/musl installer wall
  (separate tickets from the same test).
- Whether pixelflux SHOULD hold an exclusive flock on its lock file: arguably
  yes, but scoot must not depend on hosts sharing its locking scheme — a
  live socket must never be unlinked on lock evidence alone.
- Bare `scoot: Invalid argument (os error 22)` starting `--headless` with the
  default IPC path on Docker Desktop's `fakeowner` volume (`/config`);
  `--socket /tmp/...` or a normal-fs `XDG_RUNTIME_DIR` works. Probably
  fs-specific; noted in the field report, file separately if it reproduces
  on a real fs.
