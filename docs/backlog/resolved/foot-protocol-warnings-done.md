---
title: "The four protocols `foot` warned about: cursor-shape, xdg-activation, toplevel-icon, text-input/input-method — DONE"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# The four protocols `foot` warned about — DONE

~~Starting `foot` under flexwm prints four warnings about protocols the
compositor does not offer~~ — DONE, all four implemented together (issue #40).

## What landed

- **`wp-cursor-shape-v1`** (version 2). `CursorShapeManagerState`, plus
  `TabletSeatHandler` (which the protocol's dispatch is bounded on whether or
  not a compositor offers `zwp_tablet_manager_v2` — flexwm does not, so no
  client can construct the tablet half). Smithay routes `set_shape` straight
  into `SeatHandler::cursor_image` as a `CursorImageStatus::Named`, the same
  path `wl_pointer.set_cursor` with no surface already took.

  The substance is on flexwm's side, and it took two goes. Advertising the
  protocol without answering it well is a *regression*: a client that had
  been uploading a proper I-beam from its own xcursor theme switches to
  cursor-shape and gets whatever the compositor draws. The first version
  answered with ten procedurally-drawn shapes (`cursor/shapes.rs`), which is
  far better than one blob triangle but still line art where the client used
  to show real artwork. **The user caught this from the other end** — "when
  we mouse over gtk it shows a real cursor" — which also exposed a framing
  error this project had been carrying:
  `docs/backlog/resolved/cursor-theme-name-done.md` (then under
  `rendering/`) recorded per-shape cursors as
  blocked on a license-clean asset, conflating *shipping* a theme (genuinely
  blocked) with *reading the one already installed on the user's machine*
  (never blocked, and what sway/niri/anvil do). So `cursor/theme.rs` now
  loads the machine's own theme through the MIT-licensed `xcursor` crate and
  draws its artwork, with the ten drawn shapes as the fallback for a machine
  that has none — a webtop or minimal container, which is a first-class
  flexwm target. `[appearance] cursor_theme` names one explicitly;
  `XCURSOR_THEME`/`XCURSOR_SIZE` are exported to children so clients that
  still load a theme themselves match.

  The drawn fallback set is not a throwaway: each shape is stamped into a
  coverage mask by a small
  rasterizer (`stroke_line`/`fill_triangle`/`stroke_circle`) and inked with
  an 8-way dilated outline; the arrow keeps `cursor.rs`'s own byte-identical
  generator, because its outline is drawn *inside* a solid body while every
  shape here is line art one or two pixels thick. All ten are built once at
  startup, indexed by an array slot at render time — no allocation on the
  render path, and stable buffer `Id`s for the damage tracker.

- **`xdg-activation-v1`** (version 1). `compositor/activation.rs`. Two policy
  bounds, both on the token: valid for 30s, and at most 64 unredeemed tokens
  at once with expired ones swept first (`get_activation_token` is
  unauthenticated and unlimited, and nothing upstream prunes the table — the
  same resource-exhaustion family as the `wl_shm` pool cap). A redeemed token
  is removed whether honored or not. Activation runs through `State::act`,
  so it inherits the session-lock gate and the core's scroll-into-view.

- **`xdg-toplevel-icon-v1`** (version 1). `compositor/toplevel_icon.rs`.
  Stores nothing: the icon lives in the surface's own double-buffered state,
  and `State::icon_name_of` resolves the *current* one when asked — so it can
  never report an icon the client attached but has not committed, and there
  is no second copy to invalidate on unmap/destroy. Exposed as
  `WindowSnapshot::icon` (optional on the wire, so no `PROTOCOL_VERSION`
  bump). No preferred icon sizes are advertised: flexwm draws no icon, so it
  has no size to prefer, and an empty list is the protocol's own way to say
  so.

- **`text-input-v3` + `input-method-v2`** (version 1 each).
  `compositor/input_method.rs`. Smithay owns the middle of this — text-input
  focus follows keyboard focus inside its own seat, and preedit/commit
  traffic is forwarded between the two protocols. What flexwm owns is the
  IME popup: tracked as a `PopupKind::InputMethod` against whichever surface
  holds the field, which is what makes both `Window` and `LayerSurface`
  render it (and send it frame callbacks) with no render-path change at all.
  `parent_geometry` answers for a layer surface as well as a window, because
  keyboard focus in flexwm can be on a launcher's search field.

## What independent review caught

Two blocking findings, both confirmed by reproducing them rather than taken
on report, and both fixed here.

1. **The IME popup was placed wrongly whenever its parent was a layer
   surface** — the exact case `input_method.rs`'s own doc, the README and
   this record all singled out as the reason `parent_geometry` answers for
   two kinds of surface. `LayerMap::layer_geometry` is the surface's
   rectangle *plus its position on the output*, but the pinned rev's layer
   render path (`space/wayland/layer.rs`) subtracts what `parent_geometry`
   returns without adding the position back — `headless.rs::layer_elements`
   has already placed the surface there. The window path
   (`space/wayland/window.rs`) *does* add it back, which is why the same
   value is right for a window and wrong for a layer surface. A launcher
   anchored centre on a 1600x1000 output would have put its candidate window
   in the top-left corner of the screen, ~600px from the search field —
   invisible only for a surface at the origin, i.e. a top-left bar. Fixed by
   returning `LayerSurface::geometry` (surface-local). Pinned by
   `a_layer_surfaces_parent_geometry_is_surface_local_not_its_place_on_the_output`,
   which asserts against *both* candidate answers and was checked to fail
   against the old code.

2. **A client-triggerable compositor panic, newly reachable because this work
   advertises `xdg_toplevel_icon_manager_v1`.** Reproduced live:
   `create_icon` → `set_name` → `set_icon` → `set_name` aborts the process
   with `assertion failed: !self.is_immutable()`. Same shape as the
   `wl_shm_pool.resize(0)` bug `dispatch.rs` already guards — the pinned rev
   posts the `immutable` protocol error and then *falls through* into
   `set_icon_name`, whose `debug_assert!` fires; `add_buffer` is a second
   trigger. `debug_assert!` only, so release builds are safe — but every
   build this project develops, tests and smoke-tests with is a debug build,
   and a buggy toolkit reaches it as easily as a malicious client. Fixed as a
   fourth guard in `dispatch.rs`, which has to track the assignment itself
   (`State::frozen_icons`) because upstream's `constructed` flag is private
   with no accessor. Both arms covered by tests that assert the *compositor*
   survives, which is the property that matters to every other client.

Non-blocking findings were acted on too: the lock-screen limitation the review
spotted is now filed
(`docs/backlog/protocols/ime-popup-over-lock-screen.md`) and documented
honestly in `input_method.rs` rather than described as "placed at the
origin", and a misleading README table row was corrected. One was
deliberately not acted on: the review asked for a test pinning "activation is
refused while the session is locked" directly rather than transitively
through `State::act`'s gate. Locking a session in a test needs a real
`ext-session-lock-v1` client, which `session_lock/tests.rs` has and does not
export; duplicating ~120 lines of it here was judged out of proportion to a
claim that holds by construction (activation calls `act`, whose gate is
covered where it lives). Recorded rather than silently skipped.

## Evidence

Measured against this branch's tree, in the session container (Ubuntu 24.04,
`foot` 1.16.2, a debug build), not narrated:

- `cargo test --workspace`: 477 + 68 + 26 pass, 1 ignored (`shape_art`, which
  prints the shapes as ASCII art for a human and asserts nothing).
  `cargo clippy -p flexwm --all-targets -- -D warnings` and
  `cargo fmt --check -p flexwm` are clean.
- `scripts/smoke-test.sh` (`--headless`, `FLEXWM=target/debug/flexwm`): exit
  0, zero `BUG` lines, including the decoration pixel checks.
- Cursor theme, end to end: with `adwaita-icon-theme` installed the
  compositor logs `loaded a cursor theme theme=default size=16`, and
  rendering every named shape through the real `Cursor::element` +
  `PixmanRenderer` path produces the theme's own artwork — including a real
  hand for `pointer` and a real busy cursor for `wait`, neither of which the
  drawn set has. Pointed at a theme name that does not exist, the same code
  falls back to the drawn shapes. Both rendered to PNG and compared by eye.
- What real clients actually do with the protocol, measured with
  `WAYLAND_DEBUG=1` against a live headless flexwm rather than assumed:
  **GTK4 4.14.5 binds `wp_cursor_shape_manager_v1` but never calls
  `get_pointer` on it** — it still uploads its own 32x32 cursor surface
  (`wl_pointer@17.set_cursor(8, wl_surface@21, 0, 0)`), so for that version
  the client path is unchanged either way. `foot` is the client that warns
  about the protocol's absence and therefore the one that switches. Recorded
  because it bounds how much the theme work changes *today*: it is insurance
  against the regression, not a visible change for every client.
- Before/after on the *actual* warnings, same `foot`, same probe (start
  headless, `msg action spawn foot`, grep the log):

  ```
  === BEFORE (main, f0463f9) ===
  warn: wayland.c:1503: no XDG activation support; bell.urgent will fall back to coloring the window margins red
  warn: wayland.c:1512: no server-side cursors available, falling back to client-side cursors
  warn: wayland.c:1523: text input interface not implemented by compositor; IME will be disabled
  === AFTER (this branch) ===
  (none of the four warnings)
  ```

  `foot` 1.16.2 does not warn about `xdg-toplevel-icon` at all (that warning
  comes from the newer `foot` in the dev VM, which is what the issue
  reported); the global is implemented and covered by its own live-client
  tests either way.

### Performance

The only path this change touches that runs per frame is `Cursor::element`
(and only under `--tty`, the one backend that draws a cursor at all). Release
build, 10 000 iterations per run, three runs each side, same container:

| | `main` (f0463f9) | this branch |
| --- | --- | --- |
| `Cursor::element`, warm | 173 / 186 / 208 / 210 ns | 187 / 218 / 233 / 207 ns (default), 190 / 217 / 243 / 196 ns (text) |

Run-to-run spread is ±35 ns on both sides and the two ranges overlap
throughout, so there is no measurable per-frame difference — which matches
what the diff does there: one extra `match` arm, a jump-table lookup
(`Shape::for_icon`) and an array index, replacing a direct field read. For
scale, ~200 ns is 0.001% of a 16.7 ms frame.

What did get more expensive is startup, once, by construction — ten bitmaps
instead of one:

| `Cursor::new` | `main` | this branch |
| --- | --- | --- |
| default 16px | 853 ns | 16.8 µs |
| 48px | 1.5 µs | 118 µs |
| 256px (`MAX_CURSOR_SIZE`) | 49 µs | 4.9 ms |

16 µs at the default is not worth a second thought. The 4.9 ms at the largest
size a config can ask for is recorded rather than hidden: it is one-time, at
startup, only for a session that configures `cursor_size = 256`, and it buys
ten shapes that are then free forever (built once, indexed by array slot, no
allocation on the render path, stable buffer `Id`s for the damage tracker).
Building them lazily would trade that for a first-use allocation while the
pointer is moving, which is the worse moment.

### `--tty` hardware bug-bash (2026-09-16, dev VM, at `e16aada`)

The session container that did the work above has no DRM device, so the gap
it left open — the cursor is the one thing only `--tty` draws — is closed
here, on the project's dev VM (`ssh -p 2222 dev@localhost`, NixOS aarch64,
real virtio-gpu/DRM/KMS via QEMU). Binary freshness checked against
`vm/README.md`'s own gotcha before trusting any run
(`ls -l --time-style=full-iso /var/cargo-target/debug/flexwm` newer than
`git log -1 --format=%ci`) — the first `scripts/smoke-test.sh` pass here
actually caught a stale binary this way (a leftover build from before this
branch was checked out still showed all four warnings; a plain `cargo build`
fixed it and the rerun showed none). Standing evidence, re-run from scratch
rather than trusted from the earlier session:

- `XDG_RUNTIME_DIR=/tmp/xdgrt cargo test --workspace`: 477 + 68 + 26 pass, 1
  ignored — matches this doc's earlier count exactly. `cargo clippy
  --workspace --all-targets -- -D warnings` and `cargo fmt --check --all`:
  clean.
- `scripts/smoke-test.sh` (`--headless`): exit 0, no `BUG` lines, decoration
  pixel checks pass (after the rebuild above).
- The issue's own acceptance grep (`--headless`, spawn `foot`, grep the log
  for the four warning strings): no matches. `no cursor theme found; using
  the compositor's own drawn shapes theme=default` confirms the fallback
  path — this VM ships no cursor theme by default (only `hicolor`/`locolor`
  icon dirs, no `cursors/`).
- **`--tty -- foot`, real DRM master.** Verified genuinely held, not just
  logged (`vm/README.md`'s own recipe): `sudo cat
  /sys/kernel/debug/dri/0/clients` shows `master=y` on seatd's fd, and `sudo
  cat /sys/kernel/debug/dri/0/state` shows `plane[33]` scanning out
  `allocated by = flexwm`. `flexwm msg pointer move` + `flexwm msg
  screenshot` over IPC, cropped and upscaled around the pointer:
  - No theme configured: I-beam over `foot`'s grid and arrow over the
    background are the drawn line-art shapes — matches the fallback
    contract exactly (this VM has none installed).
  - `nix build nixpkgs#adwaita-icon-theme`, symlinked to `~/.icons/Adwaita`,
    `[appearance] cursor_theme = "Adwaita"`: log line `loaded a cursor theme
    theme=Adwaita size=16`; both crops now show real Adwaita artwork
    (anti-aliased hollow-bar I-beam, a proper angled pointer arrow) —
    visibly distinct in shape and shading from the drawn fallback captured
    moments before, not just a theme-name log line taken on faith.
  - `flexwm msg windows` carries `"icon": "foot"` — `xdg-toplevel-icon-v1`
    exercised end to end by a real client on real hardware.
  - Spawned a second `foot`: two-column tiling and both focus-ring colors
    render correctly under `--tty` — no regression to the ordinary path.
  - `grep -iE "panic|error"` on both `--tty` run logs: no matches beyond the
    pre-existing, unrelated `Failed to destroy old mode property blob: No
    such file or directory` (a legacy-fbadd quirk of this VM's virtio-gpu,
    seen on `main` too, not introduced here). **Not fully reproducible**: a
    reviewer's independent spot-check of this same scenario also saw, only at
    teardown, `Failed to restore previous state. Error: Permission denied`
    from the same DRM device-teardown path — present on `main` too (this PR's
    diff does not touch that code) but not something either run here can
    claim never happens; recorded honestly rather than smoothed over.
  - Both runs shut down cleanly via `flexwm msg action quit`; seatd logs
    confirm the seat was released each time (no process left holding it for
    whoever runs `--tty` next).

**Not exercised even now:** VT switching (this bug-bash never had a second
VT to switch to/from over ssh) and `xdg-activation-v1` end to end (no
activation-aware launcher was on hand). Both are called out here rather than
silently assumed clean — the ordinary render/input/tiling path they'd
interact with was exercised extensively above and showed no regression.

### The two remaining gaps, closed after merge (2026-09-16, same dev VM)

Both of the items directly above turned out to be testable on this same VM
without new hardware, once looked into rather than assumed unreachable —
`chvt`/`openvt` are already on the system, and `nix build` can fetch a real
launcher just as it fetched Adwaita earlier in this doc:

- **VT switching, `--tty -- foot`.** `sudo chvt 3` away from the VT flexwm
  was bound to: log shows `session paused; drm master released` immediately,
  libinput devices suspended, and `/sys/kernel/debug/dri/0/clients` confirms
  `master=n`. IPC (`msg version`) still answers while paused — the event loop
  itself doesn't block, only the DRM render path does. `sudo chvt` back:
  `session activated`, a forced full modeset (`drm: modeset (full commit)`),
  no `could not reactivate`/`could not resume libinput` warnings, master
  back to `y` with a flexwm-allocated `fb` on the scanout plane. Spawned a
  second `foot` afterward: two-column tiling and both focus rings rendered
  correctly, proving the render/input path is actually live again, not just
  the process. This exact scenario surfaced a second, independent, *pre-existing*
  finding — see `docs/backlog/resolved/drm-teardown-restore-eperm-done.md`.
- **`xdg-activation-v1`, end to end, with a real launcher.** `nix build
  nixpkgs#fuzzel` (1.14.1) — confirmed via `strings` to link real
  `xdg_activation_v1`/`get_activation_token` support, unlike anything
  synthetic. Gave it one `.desktop` entry (`foot`) via `XDG_DATA_DIRS`,
  drove it entirely over IPC (`msg key Return` on the already-selected
  entry — no physical input at all), and captured `WAYLAND_DEBUG=1` output
  across the fork/exec boundary (env vars, including `WAYLAND_DEBUG` and the
  activation token, survive fuzzel's fork into `foot`, so both processes'
  protocol traffic land in the same log). Full trace: fuzzel
  `get_activation_token` + `set_serial` + `set_surface` + `commit` →
  flexwm logs `xdg-activation token created token="hGz6..."` → fuzzel forks
  and execs `foot` with that token in `XDG_ACTIVATION_TOKEN` → **`foot`
  itself**, now a separate live client, opens its own connection, creates
  its real toplevel surface, and calls `xdg_activation_v1.activate(token,
  its_own_surface)` → flexwm logs `activating a window id=WindowId(1)
  app_id=None` and `flexwm msg windows` confirms that window `"focused":
  true` immediately after, with fuzzel's launcher UI gone from the
  screenshot. Every hop of the real protocol flow the module doc describes,
  exercised by two independent real clients, not a synthetic in-process
  test.

### Independent review of `e16aada` (the commit the earlier review never saw)

The "What independent review caught" section above reviewed `b312366` --
before `cursor/theme.rs` (264 lines) and its tests (169 lines) existed. A
second independent pass, dispatched specifically because of that gap,
reviewed the full diff through `e16aada` (plus this doc's own hardware-bug-bash
commit, doc-only). Verdict: no blocking findings. It independently re-derived
correctness for `frozen_icons` (traced every write/read site and the two
Smithay fall-through arms it guards), the layer-surface `parent_geometry` fix
(traced the popup-offset math for both the window and layer render paths),
the activation token bounds' enforcement, and the absence of any
theme-parsing panic path against malformed/adversarial xcursor files -- and
live-spot-checked a themed `--tty` run itself rather than trusting the
evidence above on faith. It also caught two doc-accuracy problems this PR
introduced, both fixed as a follow-up commit before merge: `cursor.rs`'s
module doc still said "there is no `cursor_theme` field here" after
`e16aada` added exactly that, and `activation.rs`/`README.md` described the
token-lifetime/count bounds as the answer to focus-stealing when they are
resource bounds only -- the real gap (no input-serial check) was stated
honestly and filed, and has since been closed:
`docs/backlog/resolved/activation-serial-validation-done.md`.

## What this deliberately leaves open

- **Toplevel icon *buffers*.** Only the icon name is exposed. A client may
  supply raw square `wl_shm` buffers instead, which
  `ToplevelIconCachedState::buffers` holds and nothing reads; handing those
  to an IPC client means re-encoding shm to PNG per query, the way
  `screenshot.rs` does for the screen, and no consumer has asked yet. Such a
  client reads as having no icon.
- **No `XDG_ACTIVATION_TOKEN` when flexwm spawns.** `Action::Spawn` does not
  mint an external token for the child, so a compositor-spawned app cannot
  activate itself the way a launcher-spawned one can. `create_external_token`
  is the upstream hook for it.
- **HiDPI cursor sizing.** Theme images are picked at the nominal
  `cursor_size` and drawn at buffer scale 1, so a scaled output gets a cursor
  at its logical size rather than the theme's larger variant. Pre-existing
  (the drawn shapes have always worked this way), but now *fixable*, since
  there is a theme with several sizes to pick from.
- **An IME popup over a lock screen is tracked but never drawn**
  (`docs/backlog/protocols/ime-popup-over-lock-screen.md`), because the
  locked render path replaces the element list wholesale by design.
- **Cursor shapes beyond the ten**, in the no-theme fallback. `help`, `wait`, `progress`, `pointer`,
  `zoom-in`/`zoom-out`, `alias`, `copy` and `context-menu` all draw the
  arrow, deliberately: a hand or an hourglass is not distinguishable as line
  art at the default 16px, and drawing a near-duplicate nobody can read is
  worse than the honest fallback.
