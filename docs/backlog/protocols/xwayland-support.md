---
title: "XWayland support (clears the Not-yet X11 bullet)"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# XWayland support

`README.md` names it ("No XWayland. X11-only applications do not run.")
with no promise attached, and no entry has ever scoped it. This entry is
that scope — file it so the bullet has a costed landing spot, not a
commitment. Nothing here is macOS-relevant (Linux-only `compositor/`
shape throughout; `scootctl` untouched).

## Verified starting point

- No XWayland in tree: Smithay features are desktop/wayland/pixman/gl/
  drm/libinput/udev/libseat (`crates/scoot/Cargo.toml:143-152`); `rg
  xwayland` hits only the intended tripwire
  (`compositor/render/elements.rs:362`, exhaustive `WindowSurface::Wayland`
  match) plus `docs/protocols.md:49` and the README rows.
- No Smithay bump needed: pinned `smithay 0.7.0` (`Cargo.lock:905`)
  carries `src/xwayland/` (server, XWM, surfaces, dnd, selection) and
  `src/wayland/xwayland_shell.rs` + `xwayland_keyboard_grab.rs`.
- Addition pattern per `linux-dmabuf-advertisement-done.md` /
  `layer-shell-done.md`: new `compositor/<proto>.rs` + `tests.rs` on the
  shared `Harness`, state field in `state.rs`, handler in `handlers.rs`,
  dispatch guards in `dispatch.rs`, README table + `protocols.md` section,
  `smoke-test.sh` wire assert.

## Phase 0 — spike, before committing to shape

Enable `xwayland` on a scratch build (do not commit): confirm the break
sites (`render/elements.rs:365`, `shell.rs:20,143-149`
`window.toplevel() == None` for X11, `handlers.rs:286`), macro names
(`delegate_xwayland_shell!`, `XWaylandShellHandler`), `XWayland::new` +
`start()` shape (`xserver.rs:89-143` — binary must be on `PATH`). Measure
with real X clients: map/unmap storms, override-redirect menus,
`WM_CLASS`/`WM_NAME` timing, `NET_ACTIVE_WINDOW` steal attempts. Decide
always-on vs opt-in going in — recommendation: opt-in flag + config key
(X server + abstract socket is not zero-cost; matches the `gpu-scanout`
precedent of never changing default sessions).

## Phase 1 — skeleton

`xwayland` in Smithay features (own cargo feature mirroring `gpu-scanout`
if opt-in; re-prove the `ldd` gate, `README.md:252`). New
`compositor/xwayland.rs`; `state.rs` fields next to
`xdg_shell_state:688`; server start in `compositor/mod.rs::run:188-227`
after `headless::init_named`, before `WAYLAND_DISPLAY` set (ordering
matters — `DISPLAY` for spawned children, no host-Wayland clobber).
`impl XWaylandShellHandler` (+ keyboard-grab) in `handlers.rs`; dispatch
coverage per the file's guard pattern. Export `DISPLAY` to `State::spawn`
children (`session_env.rs`, `mod.rs:238-279`); X children get `DISPLAY`
only, no activation token.

## Phase 2 — window mapping into scoot-core

`shell.rs` (`add/remove/refresh/info_of/apply/act`) gains `X11Surface`
parallels via `Window::new_x11_window`; `WM_CLASS`→app_id,
`WM_NAME`→title, `WM_NORMAL_HINTS`→`SizeHints` through
`clamp_hint/hint_limit:379-394`. Policy: ignore X11 self-positioning —
every mapped X window is a column entry; override-redirect windows never
enter the core (unmanaged). Foreign-toplevel lists announce from the same
points, so X windows appear in `scoot msg windows` + both protocols,
additive, no version bump. `render/elements.rs` gains the `X11` arm;
lock, per-output bookkeeping and decorations key off `Space<Window>` and
follow for free — verify each gate tests `Window`, not `toplevel()`.

**Note for Phase 2 (from PR #230, GPU scanout tier):** `State::fullscreen_surface`
(`fullscreen.rs`, `gpu-scanout` builds) resolves the covering window through
`Window::toplevel`, which is `None` for an X11 window. So a fullscreen X11
window currently answers `NotCovered` in `render::primary_direct` and is
never scanned out directly nor sent the per-surface scanout feedback
(`dmabuf/scanout.rs`) -- composited like any other window, never wrong, but
never zero-copy. When X11 windows map, extend `fullscreen_surface` to return
the X11 surface's `wl_surface` (`X11Surface::wl_surface`), and check that
rule 6's tree walk and the steering both accept it.

## Phase 3 — input / focus gate (the security half)

Pointer hit-test and keyboard focus gain X11 branches
(`input.rs:982`, `shell.rs:refresh_keyboard_focus:227`). X11 has no
activation serials, so `map_window_request` must not steal focus
unconditionally (the `activation-serial-validation-done.md` hole in X
form): focus-on-map only if spawned-here-with-chain or nothing focused;
else announce and let `activate`/click/IPC move focus.
`NET_ACTIVE_WINDOW` is a request through the same gate, never a command.
X input still feeds `announce_activity` (idle) and `interaction_serials`
(so xdg gates keep working). Popup precedence
(lock > exclusive layer > grab > window) dismisses X menus on lock.

## Phase 4 — clipboard / DnD / IME

Smithay XWM bridges selections both ways (`xwm/selection.rs`, `dnd.rs`)
into the existing `handlers.rs:462-590` endpoints; the primary-selection
focus gate applies to the bridge — a background X client must not set it.
X-origin drags map onto the `interaction_serials` check or document why
not. XIM unsupported (X clients use it, not `text-input-v3`) — document,
ensure IM grab still pre-empts X focus.

## Phase 5–7 — capture, packaging, docs

No new capture protocol: screencopy + IPC screenshot read the composited
framebuffer, X windows appear automatically — pin with pixel tests (red X
window, per-output, lock blanking). Nix: `xwayland` server binary in
`vm/compositor-deps.nix`, session `PATH` must include it for `--tty`
logins (the `xserver.rs:89` must-be-on-PATH failure is otherwise a silent
Wayland-only session — log loudly). Docs: `protocols.md:47-50` gains the
version row + trust-model note (same-uid boundary now bites harder — X11
clients can keylog/snoop by design; running one extends full trust),
README bullet deleted and `What works:215` flipped, config + nix docs for
the knob.

## What done looks like

Full-green `nextest` both renderers, new `xwayland/tests.rs` on `Harness`
(map→tiled+listed, title/class arrival, close, activate/close via wlr
protocol, focus-steal refusal fail-first each, override-redirect
non-column, lock blanking + input refusal, clipboard round-trip,
screenshot pixels), live matrix (`--headless`/`--nested`/`--tty`, `xterm`,
`wayland-info` + `xprop` probes, smoke-test section), absent-binary
fallback proven (session starts Wayland-only, never crashes).

## Phase 0 findings (spike 2026-09-22, ticket stays OPEN)

Live spike, not static: a scratch probe compositor (`/tmp/xwspike0`, never
in the repo — the committed diff of this PR is this file only) hosting
`XWayland` + `X11Wm` at the pinned Smithay rev drove real X clients on the
dev VM (aarch64 NixOS, `ssh -p 2222 dev@localhost`, headless only, no
`--tty`). X server + clients came from nixpkgs (network to
`cache.nixos.org` works): Xwayland 24.1.13, xterm 410, xprop 1.2.8,
xwininfo 1.1.6. Raw logs: `/tmp/xwspike0-events.log` (852 lines),
`/tmp/xwspike0-results.txt` on the dev VM.

### Ticket corrections (pinned rev `0ff00983`, verified against
`~/.cargo/git/checkouts/smithay-312425d48e59d8c8/0ff0098/`)

- **No `XWayland::new` + `start()`.** The constructor is a single
  `XWayland::spawn(dh, display, envs, extra_args, open_abstract_socket,
  stdout, stderr, user_data)` (`src/xwayland/xserver.rs:115`; no `fn new`
  / `fn start` anywhere in that file). It hard-codes
  `Command::new("Xwayland")` (`:143`) — PATH-only resolution — and clears
  the environment except `PATH` + `XDG_RUNTIME_DIR` (`:165-173`).
- **No `delegate_xwayland_shell!` macro.** `grep macro_rules!
  delegate_xwayland src/` is empty. `xwayland_shell.rs:222-278` implements
  `GlobalDispatch2`/`Dispatch2` for the generic `GlobalData` /
  `XWaylandSurfaceUserData`, so the blanket `delegate_dispatch2!(State)`
  — the same seam `compositor/dispatch.rs` owns — covers the shell global
  with zero new dispatch code. The probe proved this: one
  `delegate_dispatch2!(State)` line, no per-interface impls.
- **Handler names confirmed:** `XWaylandShellHandler`
  (`xwayland_shell.rs:193`: `xwayland_shell_state()` + defaulted
  `surface_associated()`), `XwmHandler` (`xwm/mod.rs:359`: 7 required —
  `xwm_state`, `new_window`, `new_override_redirect_window`,
  `map_window_request`, `mapped_override_redirect_window`,
  `unmapped_window`, `destroyed_window`, `configure_request`,
  `configure_notify`, `resize_request`, `move_request` — plus ~25
  defaulted, including the load-bearing `active_window_request`
  (`:515-522`, default body is `let _ = ...`, i.e. refuse-by-default),
  `XWaylandKeyboardGrabHandler`
  (`xwayland_keyboard_grab.rs:70`: `grab` + `keyboard_focus_for_xsurface`).
  Smithay cargo feature is `xwayland` (`Cargo.toml:117`).
- **Window API confirmed:** `Window::new_x11_window`
  (`desktop/wayland/window.rs:198`), `window.toplevel() == None` for X11
  (`:472-476`), `x11_surface()` accessor (`:483`), `set_activated` handles
  X11 (`:287`), `surface_under` handles X11 (`:461` — pointer hit-test
  mostly free), `send_ping` handles X11 (`:315`); `X11Surface::title()`
  / `class()` / `instance()` (`xwm/surface.rs:1081-1091`),
  `is_override_redirect()` (`:442`), `wl_surface()` returns `Option`
  (`:876` — keyboard-focus path must branch, not unwrap),
  `close()` (`:1995`), `configure()` refuses a rect for
  override-redirect (`:505-509`).

### Break-site inventory (re-verified at `8b048f5`; ticket line numbers moved)

- `compositor/render/elements.rs:362-365` — the tripwire match, needs the
  `X11` arm. Still exact.
- `compositor/shell.rs` — `add_window:17-20` (takes `ToplevelSurface`,
  calls `new_wayland_window`; needs the `X11Surface` parallel),
  `refresh_window:76-83` via `info_of:319-322` (`toplevel` → `default`;
  needs `class→app_id`, `title→title`), `act` `Effect::Close:136`
  (`toplevel().send_close()`; X11 needs `x11_surface().close()`),
  `apply:164-169` (configure is `toplevel`-gated; X11 needs its own
  configure), `set_focus:198-203` (`set_activated` is X11-safe;
  `send_pending_configure` is `toplevel`-gated), `refresh_keyboard_focus:
  277-282` (`toplevel().wl_surface()`; X11 surface is `Option`).
- `compositor/handlers.rs:288-301` — `XdgShellHandler` impl site; the new
  `XWaylandShellHandler` + `XwmHandler` + keyboard-grab impls land here.
- `compositor/input.rs:968` — `space.element_under` click path works on
  `Space<Window>` once mapped; the X11 branch needed is in
  `refresh_keyboard_focus` (above), not here.
- `compositor/state.rs:310` (field) / `:695` (construct) —
  `xdg_shell_state` pattern for the new `XWaylandShellState` + `X11Wm` +
  display-number fields; `State::spawn:1045` (`WAYLAND_DISPLAY` export
  site; `DISPLAY` joins it).
- `compositor/mod.rs:189` (`headless::init_named`) / `:241`
  (`WAYLAND_DISPLAY` set) — X server start goes between, per the Phase 1
  ordering note (still correct).
- `compositor/dispatch.rs` — no new code: the blanket covers
  xwayland_shell (probe-proven); XWM is not a Wayland global.

### Measurements (raw, dev VM)

- **Startup:** `XWAYLAND_READY` at t=70ms post-spawn (preserved run;
  a second unpreserved run read 84ms); `DISPLAY=:6`. Fresh Xwayland RSS **55,184 KB** / VSZ 255,452 KB
  with one client mapped — the always-on cost, measured not estimated.
- **Title/class timing (xterm 410):** spawn → visible in
  `xwininfo -root -tree` **76ms**; spawn → `WM_CLASS` via xprop **96ms**;
  spawn → `WM_NAME` **101ms** (`WM_CLASS="xterm","XTerm"`,
  `WM_NAME="spikeA"`; `_NET_WM_NAME` absent — xterm doesn't set it).
  Compositor-side, `title()`/`class()` were already populated at all 31
  `new_window` callbacks (probe poll: 31 arrived, **0 timeouts**) — no
  async-title race to design around for the common case.
- **Map/unmap storm (30 sequential `xterm -e sleep 0.3`):**
  `new_window=30`, `map_window_request=30`, `unmapped=30`,
  `destroyed=30`, `configure_notify=30` — zero event loss, order sane per
  window. `unmap→destroy` **0–1ms**. `new→map` median **5028ms**
  (min 5019, max 5034) — this is xterm's own startup (every spawn prints
  `cannot load font "-misc-fixed-...";` the nix xterm lacks its bitmap
  fonts and falls back slowly), **not** compositor latency: the pure-x11rb
  client below goes `new→map` in **1ms**. ~2 `configure_request` per
  window (10×17 initial, then 484×316 real) and ~13 `property_notify`
  (NormalHints, Pid, Protocols, …) — the Phase-2 mapping code should
  expect configure + property chatter per window, not one shot each.
- **Override-redirect (x11rb client, W2=4194305):**
  `new_override_redirect_window=1`, `mapped_override_redirect_window=1`
  (same ms), `map_window_request` for W2 = **0** — routing is exactly the
  ticket's Phase-2 policy with no extra work. Normal sibling W1=4194304:
  `new→map` **1ms**, `map_window_request=1`.
- **`NET_ACTIVE_WINDOW` steal (client message, source=pager, for W1):**
  `active_window_request` fired once; probe refused (logged, no action);
  root `_NET_ACTIVE_WINDOW` read `none` before and `none` after via
  `xprop` — steal attempt observable **and** refused through the default
  no-op. Fail-first test shape for Phase 3 exists.

### Integration hazards found live (all three bit the probe first)

1. **`dispatch_clients` does not flush.** The probe's loop dispatched but
   never flushed; Xwayland sat in `ppoll` on `get_registry` (strace-proven:
   one 24-byte send, zero recvs, empty stderr, no log file — a silent
   hang, not an error). Fix was one `flush_clients()` after dispatch.
   scoot flushes in its render loop so this is a non-issue in-tree, but
   any headless Xwayland test harness needs the explicit flush.
2. **`client_compositor_state` must serve `XWaylandClientData`.** The
   probe panicked (`no per-client compositor state`) on the first X
   surface commit; fix is the anvil pattern (`shell/mod.rs:106-114`):
   check `XWaylandClientData` first, then own client data. Confirms X
   windows *do* commit wl_surfaces (association path is live).
3. **Absent binary is a loud spawn failure, not a hang** (`XWayland::spawn`
   returns `Err` when `Xwayland` is not on `PATH`) — the Phase-1 fallback
   (log loudly, Wayland-only session, never crash) is straightforward to
   pin.

### Decision: opt-in flag + config key (ticket recommendation ACCEPTED)

Always-on is rejected on measured grounds: +55 MB RSS and a whole X
server process on every session including GPU-free webtop ones; a hard
`PATH` dependency for the binary; and an X11 trust-model change (any X
client can keylog/snoop by design — running one extends full trust to
it) that must be an explicit choice. Precedent is `gpu-scanout`: own
cargo feature (`xwayland-server = ["smithay/xwayland"]`, default off,
`ldd` gate re-proven), CLI flag plus `[xwayland] enabled` config key,
default sessions byte-identical.

### Scoped Phase 1 work list (skeleton only; mapping/policy stay Phase 2+)

1. `crates/scoot/Cargo.toml` — `xwayland-server` feature mirroring
   `gpu-scanout` (comment block included); `ldd` proof both flavours.
2. `compositor/xwayland.rs` (new) — start/stop around `XWayland::spawn`,
   `open_abstract_socket=true`, READY→`X11Wm::start_wm`, opt-in gate.
3. `compositor/state.rs` — `XWaylandShellState` + `Option<X11Wm>` +
   display-number fields (`:310`/`:695` pattern); `DISPLAY` export in
   `spawn` (`:1045`).
4. `compositor/handlers.rs` — `XWaylandShellHandler` + `XwmHandler`
   (log + refuse defaults; map/OR routing is Phase 2, only the refusal
   half lands here) + `XWaylandKeyboardGrabHandler`.
5. `compositor/mod.rs` — start between `:189` and `:241`;
   spawn-failure → loud log + Wayland-only (fail-first test: `PATH`
   without `Xwayland`).
6. `compositor/session_env.rs` + config + docs — `DISPLAY` plumbing and
   the knob (`--xwayland`, `[xwayland] enabled`, default off).
7. `smoke-test.sh` wire assert + `protocols.md` version row (deferred
   content per Phase 5–7 stays deferred; only the skeleton rows land).
8. `compositor/shell.rs` X11 branches, `elements.rs` X11 arm, and all of
   Phase 2–4 are explicitly NOT Phase 1.

### Go / no-go: CONDITIONAL GO

No technical blocker: the API exists at the pinned rev, the event flow
is measured end to end, the steal gate refuses by default, and the cost
is quantified (~70ms startup on the preserved run, ~55 MB RSS, ms-scale event path).
Top 3 risks for Phase 1+: (1) focus/activation policy — X11 has no
serials, so `map_window_request` focus and every `active_window_request`
must default-refuse (the exact hole `activation-serial-validation`
closed for xdg, now in X form); (2) XWM threading/reentrancy —
`X11Wm`'s callbacks run on the loop thread but its x11rb connection is
independent; audit against `Space` borrows before mapping anything;
(3) `--tty` packaging — the `PATH` requirement becomes a silent
Wayland-only session on logins unless `vm/compositor-deps.nix` carries
the binary and the fallback logs loudly (hazard 3 above is the pin).

## PROGRESS — Phase 1 (skeleton) landed; ticket stays OPEN for Phase 2+

Phase-1 PR: **#221 (`xwayland-phase1-skeleton`, commit `772dbc0`)** — open
for review, not merged by the implementer. Scope keeps
exactly to the 8-item list above: own `xwayland` Cargo feature (named
`xwayland`, not the sketch's `xwayland-server`), `compositor/xwayland/`
around `spawn` + `open_abstract_socket=true` + READY→`start_wm` with both
hazard fixes, `state.rs` fields, `handlers.rs` impls
(`active_window_request` deliberately unimplemented — the default no-op
*is* the steal refusal, spike-verified), `mod.rs` start between
`init_named` and the `WAYLAND_DISPLAY` export with loud Wayland-only
fallback, `DISPLAY` plumbing + knob (`--xwayland`, `[xwayland] enabled`,
OR-ed, default off), reload refusal (`xwayland.enabled`, restart-named),
smoke section, `protocols.md` skeleton rows + trust note. No
`PROTOCOL_VERSION` bump (no wire change: no new IPC request, reply, or
action — the knob is CLI/config surface only). `shell.rs`/`elements.rs`
mapping and Phases 2–4 explicitly not started: X clients connect, get a
display, and map nowhere.

Two deviations from the sketch worth recording: (1) the knob parses in
*every* build flavour (the `--renderer gles` precedent) — a non-`xwayland`
build warns once and runs Wayland-only rather than refusing a flag its
`--help` advertises; only the Smithay types sit behind the feature gate.
(2) `State::spawn` mints activation tokens exactly as before in Phase 1
(no launch path can distinguish X children yet), so the "X children get
`DISPLAY` only, no token" rule lands as documented policy constraining
Phase 2+'s launch path, not as a spawn branch today.

Phase-2 entry points, measured during Phase-1 verification: the XWM is a
*reparenting* manager (test windows move out of the root's children into
frames — an `xwininfo -root -tree` probe for a mapped X window must walk
one level down); test-log capture needs an explicit DEBUG floor (the
compositor's production default is INFO, the handler refusals log at
DEBUG); `/var/cargo-target` on the dev VM fills fast with per-flavour
test binaries (freed 10.8 GiB with `cargo clean -p scoot` mid-branch —
shared cache, every agent rebuilds only `scoot` itself afterwards).

## PROGRESS — Phases 2 + 3 (mapping + focus gate) landed together; ticket stays OPEN for Phase 4+

Shipped as one PR (2026-09-25, PR #244) because either half
alone is unsafe or useless: mapping without the gate hands X clients the
focus-steal hole, the gate without mapping has nothing to gate.

**The keyboard needed a focus-type change first (measured, not assumed).**
XWayland does not move the X server's input focus on `wl_keyboard.enter`:
with the seat focused on an X window's `wl_surface`, `GetInputFocus` read
`PointerRoot` (`0x1`) before and after and an injected key reached neither
of two mapped X windows; only Smithay's `X11Surface` keyboard target
(`SetInputFocus` + `WM_TAKE_FOCUS` per ICCCM input model) moved it
(`0x400000`). `PointerRoot` sends keys to whatever X window is under the X
pointer -- a keylogging hole the compositor would open. So
`SeatHandler::KeyboardFocus` became `keyboard_focus::KeyboardFocus`
(`Surface(WlSurface)` in every build, `X11 { window, surface }` under the
feature; the X arm carries its surface so the pointer-focus conversion the
popup grab needs stays total). Landed alone first, behaviour-neutral, all
flavours green.

**Phase 2 (`xwayland/manage.rs`, `unmanaged.rs`):** a map request is
granted and the window enters the core (`WM_CLASS` class → app id,
`_NET_WM_NAME`/`WM_NAME` → title, `WM_NORMAL_HINTS` → hints through the xdg
clamp, `WM_TRANSIENT_FOR` → parent); it leaves on unmap/destroy (by X id --
Smithay marks a destroyed surface dead before `destroyed_window`, so `==`
never matches) and on the server's death (a sweep: a dead server sends no
unmaps). Float policy updated for floating windows (#242/#243): transients,
any non-`NORMAL` window type, `_NET_WM_STATE_MODAL` and fixed sizes float
through `window_rules.rs`, centred on an X parent; rules match the class. A
floating X window keeps a `USPosition`/`PPosition` that fits one output's
usable area, else centres; later size requests are floating resizes; tiled
windows' requests get Smithay's synthetic notify. `apply()` configures
visible X windows at their placement (clamped to `INT16`/`CARD16`, and only
when the rect changed) and keeps `_NET_WM_STATE_FULLSCREEN` in step both
ways. Override-redirect windows are drawn unmanaged above windows (below
top/overlay), hit-tested there, given frame callbacks and presentation
feedback -- all behind the lock branches. `fullscreen_surface` now returns a
`Cow`, so a fullscreen X window's surface is eligible for direct scanout and
steering (rule 6 walks it like any root). No X "tiled" state is set;
Motif hints are ignored (scoot draws no titlebar); X clients draw at scale 1.

**Phase 3 (`xwayland/focus.rs`):** focus on map only if nothing is focused,
the focused window is an X window of the same client process (X-Resource
pid -- a deliberate addition to the brief: without it a GTK app's file
chooser opened unfocused, measured with `mousepad`, because GTK sends no
`_NET_ACTIVE_WINDOW` for a new dialog), or the window redeems its spawn's
activation token -- by `_NET_STARTUP_ID` on the window or, failing that,
on its client leader (`WM_HINTS` window group, where GTK and Qt put it)
(`State::spawn` now exports the token as `DESKTOP_STARTUP_ID` too while
XWayland is live; this reverses Phase 1's documented "must not mint", which
predated any X redemption path) or by its X-Resource client pid being an
unreaped spawned child with a live token (covers `xterm`, which sets no
startup id; `_NET_WM_PID` is never read). A live token the window carries
is spent whichever rule grants focus. `_NET_ACTIVE_WINDOW` passes the
same gate, refused while locked, cheap checks before any round trip, pid cached per window. An
input-model-`None` window gets no keyboard; the focused X window is raised in
X stacking; the keyboard-grab protocol stays refused. X input feeds idle and
`interaction_serials` through the same paths as Wayland input.

**Review round 1 (same PR):** an over-long X title or class (Smithay reads
8192 bytes; `STRING` values decode from Windows-1252, 0x80-0x9F to three
UTF-8 bytes) disconnected every foreign-toplevel watcher, repeatedly -- X
strings are now also capped at 4000 UTF-8 bytes after decoding, walked back
to a character boundary (the hard wire bound is 4083); rule 1 and rule 3 no
longer short-circuit past the token (a copied token redeemed later took
focus from a Wayland window, live); the startup id is read from the client
leader too (GTK's wrapped launches got focus by pid only, never by startup
id); an override-redirect window mapped during the lock is pinned hidden and
inert; CI builds, lints and runs the live suites with `Xwayland` installed
and `SCOOT_REQUIRE_XWAYLAND=1`, so a skipped live test fails there.

**Known limits, measured and stated rather than hidden:** (1) the startup-id
chain is copyable before the app maps -- `_NET_STARTUP_ID` is readable from
the moment a toolkit sets it on its leader, so a watching X client can map
a window with a copy (or naming the leader as its group) before the app's
own window maps, within the token's 30 s, and take focus once; binding the redemption to the spawned process (ppid walk) is
filed as [`xwayland-startup-id-race.md`](./xwayland-startup-id-race.md). (2)
The brief asked the lock to *dismiss* X menus; it hides them and refuses
them input, but cannot close them: the WM cannot unmap an override-redirect
window and GTK 3 keeps its context menu through the focus release (measured:
still mapped 5 s after locking), so a menu open at lock reappears at unlock.

**Hostile X properties** (found while building it, each reproduced first):
a NUL in `WM_NAME`/`_NET_WM_NAME`/`WM_CLASS` reached a foreign-toplevel
`title`/`app_id` and panicked the compositor (`NulError` in
wayland-scanner's `CString::new(..).unwrap()`) -- X strings are now cut at
the first NUL; `_GTK_FRAME_EXTENTS` near `i32::MAX` overflowed Smithay's
`Rectangle - FrameExtents` in `X11Surface::geometry()` (a debug panic,
garbage in release) -- a window with an extent past 32767 is refused at map
and withdrawn if it grows one. A Smithay-side clamp (the fork, then
upstream) would let such a window be managed rather than refused; not
needed for safety.

**Evidence** (details and raw logs in the PR): 33 `xwayland::tests` on `Harness` (32 in every `xwayland` build -- 27 live against a real XWayland, 5 hermetic -- plus the scanout one in `gpu-scanout,xwayland` builds)
(mapping, lists, floating/centring, rules, position policy, override-
redirect, fullscreen, close, ring/rounded clip, focus gate, typing, click,
taskbar activate/close, lock blanking and input refusal, server death
sweep, WM-attach failure); the gate and lock tests were run against
mutations that disable each branch and fail at the intended assertion; live
matrix on the dev VM under `--headless`, `--nested` (cage) and `--tty`
(pixman) with `xterm`, `xeyes`, `xclock` and GTK4 `zenity` over
`GDK_BACKEND=x11` (typing reached xterm, a background `xdotool
windowactivate` was refused, a click focused it, the dialog floated
centred); default-build hot paths unchanged within noise (key 2.00 vs 2.00
µs, pointer 4.12-4.18 vs 4.16-4.35 µs, frames identical).

What stays open here: **Phase 4** (clipboard/DnD bridge through the
selection focus gate, XIM), **Phases 5–7** (capture pins, `vm/compositor-deps.nix`
+ session `PATH` packaging and a flake output for the feature). Smaller
follow-ups noted in the code: `_NET_WM_MOVERESIZE` (client titlebar drags)
not honoured, `_NET_WM_ICON` not read, X windows not scale-aware.
