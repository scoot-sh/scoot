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
