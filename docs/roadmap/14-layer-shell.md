---
item: "14"
title: "wlr-layer-shell-unstable-v1"
status: "done"
area: "protocols"
pr: 22
commit: null
---

# wlr-layer-shell-unstable-v1

**Scope.** This PR covers the protocol (global at version 5, surface
lifecycle, configure/ack, close), all four layers rendering in the right
order, anchors/margins/sizing, exclusive zones shrinking the tiling area,
pointer input, and — added in the second round, see
**Keyboard interactivity** below — `keyboard_interactivity`. Bars, docks,
wallpapers, notification daemons *and* launchers are usable. What is
still missing is popups from a layer surface, which is blocked on a
pre-existing bug (no `xdg_popup` gets its initial configure at all; see
the backlog entry).

The first round deliberately deferred keyboard focus, on the reasoning
that the zone is nearly free (Smithay's `LayerMap` computes it) while
keyboard focus means an override inside `shell.rs`'s `set_focus`, the
compositor's most safety-critical path. Review rightly pushed back on
shipping that gap: the failure mode wasn't "a launcher can't be typed
into," it was "a launcher, or a layer-shell lock screen, maps and draws
convincingly while every keystroke goes to the window behind it" — worse
than not supporting the protocol at all, because before this branch such
a client failed loudly at bind time. So it is implemented here.

**Smithay does the geometry, flexwm does the placement.**
`smithay::desktop::LayerMap` (one per `Output`) already implements the
protocol's anchor/margin/exclusive-zone rules, including the `-1`
"don't push me around" sentinel and the implied exclusive edge for a
surface anchored to three sides, and exposes `non_exclusive_zone()`.
`compositor/layer_shell.rs` owns the rest: when to re-arrange (creation,
every commit, output resize, teardown), where the results sit in the
render stack, and how the zone reaches the core.

**The render stack changed shape, and had to.** `headless.rs`'s
`render()` used `space::space_render_elements`, which gathers layer
surfaces itself in one fixed order: upper layers, windows, lower layers.
That cannot express where flexwm's focus ring goes — *between* windows
and the background layer — so a full-screen wallpaper would have been
drawn on top of the ring, hiding it entirely for anyone running `swaybg`.
`render()` now calls `Space::render_elements_for_region` (windows only,
by construction) and gathers the layers itself around the ring, giving
front-to-back: cursor, overlay, top, windows, ring, bottom, background.
`Elements`'s `Space` variant became `Surface` — windows and layer
surfaces are the same element type, so a variant each would need two
`From<WaylandSurfaceRenderElement<_>>` impls, which cannot coexist; what
orders them is insertion order, which a variant could not have expressed
anyway. The `Err` arm that logged "could not gather window render
elements" is gone with the call: at this rev `space_render_elements`
returns `Ok` unconditionally, so it was dead.

Layer surfaces also get frame callbacks (`LayerSurface::send_frame` for
every mapped one, per frame) — without them a bar's clock freezes on the
second it first drew, the same starvation item 8 fixed for cursor
surfaces — and `LayerMap::cleanup` runs in the same pass as a second line
of defence behind `layer_destroyed`.

**The core's change is one field, and its two meanings are kept apart.**
`flexwm_core`'s `tree::Output` gains `usable: Rect` beside `area: Rect`.
`area` stays the whole screen — it is what `World::outputs()` and
`flexwm msg outputs` report, and reporting a bar-shrunken rectangle there
would have told an agent the display is smaller than it is. `usable` is
what *every* layout read uses (`place_workspace`, `fix_view`,
`learn_from_frame`, and `shell.rs`'s `hint_limit` via the new
`World::usable_areas()`). The new `Event::OutputUsableAreaChanged`
intersects with `area` on the way in, and an area change re-clamps rather
than resets, so `usable` can never describe space the output doesn't
have. One subtlety found by the randomized invariant test rather than by
inspection: `Rect::intersection` reports a *non*-overlap at the later of
the two starting corners, which is a client's number — a layer surface
whose exclusive zone and margins put it at `i32::MAX` left an empty
usable area *there*, and `Rect::inset`'s `x + by` then overflowed on the
next `arrange`. An empty axis is now pinned back to the output's origin
(`tree::clamp_usable`).

**Keyboard interactivity, and why the policy is derived rather than
stored.** `layer_shell.rs`'s `LayerFocus` is the whole policy —
`exclusive` on `top`/`overlay` takes the keyboard when the surface maps
and holds it until it unmaps; `on_demand` anywhere, and `exclusive` on
`bottom`/`background` (which the spec explicitly hands back to the
compositor: "for the bottom and background layers, the compositor is
allowed to use normal focus semantics"), is click-to-focus like a window;
`none` never. `State::layer_keyboard_focus` re-reads the layer map on
every focus refresh instead of remembering a holder, which is what makes
"front-most exclusive wins", "the keyboard comes back on unmap" and "it
falls to the next exclusive surface when this one dies" fall out for free
rather than each needing its own teardown path. The one thing stored is
`State::clicked_layer`, because nothing else records that a click
happened.

Three decisions inside that are worth their own line:

- **Mapped-ness is `LayerSurfaceCachedState::last_acked`, not layer-map
  membership.** Smithay's `pre_commit_hook` maintains that field as
  exactly "has a buffer" (set from the acked configure when one is
  attached, cleared when one is removed — its own doc says "Reset to
  `None` when the surface unmaps"). Membership would have let a surface
  that commits and never draws, or that unmaps itself with a null buffer,
  sit on every keystroke with nothing of it on screen. The equivalent
  hazard for *exclusive zones* is still open (see the backlog entry) —
  there Smithay gives no such hook, here it does, and keyboard capture is
  worth being stricter about than reserved space.
- **Window focus deliberately does not move.** `arrangement.focused`, the
  ring, `set_activated` and `flexwm msg windows`' `focused` flag all keep
  naming the window — which is the one focus returns to, and is what an
  agent is asking about. Only the keyboard is overridden. This is in the
  README's agent-facing section too, because `flexwm msg type` going to a
  launcher while `flexwm msg windows` names the window behind it is
  surprising if you haven't been told.
- **Keybindings keep working**, because `input.rs`'s `key()` matches them
  before forwarding anything. That is what makes it safe to hand a
  full-screen client every keystroke: `Super+Shift+E` and the `--tty`
  `Ctrl+Alt+F<n>` VT switches are still reachable if it wedges. It is
  also why a layer-shell *lock screen* on flexwm is a screen blanker you
  can type into, not a security boundary — said plainly in the README
  rather than left to be discovered.

The hot path was kept honest: `commit_layer_surface` gates the focus
refresh on `layer_focus(&layer) != Never || self.keyboard_on_layer`, so a
`none` bar redrawing its clock at its own frame rate pays one `Copy`
snapshot and a bool rather than a second walk of the layer map. The
second half of that condition is the one that is easy to get wrong: a
surface that *stops* wanting the keyboard reads as `Never`, and without
it nothing would ever take the focus back off it. There is a test for
exactly that transition.

Be precise about what that gate buys, though, because the code comment
reads more absolute than it is: the cheap path is "a `none` bar commits
**while nothing holds the keyboard**". Once a launcher is up,
`keyboard_on_layer` is true, so every bar commit takes the second branch
and re-derives focus — which is the most likely source of the one extra
jiffy in the "ticking bar + focused launcher: 2" row below (bar alone: 1).
Tightening it to "this surface is the current holder" would recover that;
it was not done here because it is one jiffy per twenty seconds and it
would mean re-running the whole hardware capture (see the Backlog).

**A client-triggerable compositor panic, found by these tests and fixed
here.** `zwlr_layer_surface_v1.set_size` takes two **`uint`**s, and the
pinned Smithay rev converts them with a bare `as i32`
(`wlr_layer/handlers.rs:189-193`) into a `Size` whose constructor holds
`debug_assert!(w.non_negative() && h.non_negative())`. So
`set_size(u32::MAX, u32::MAX)` — one request, any client, no privilege,
no buffer — **panics a debug build of the compositor**, taking every
connected client down with it: the same family as item 7's
`wl_shm_pool.resize(0)`, and it would have shipped *with* this feature
rather than despite it. `dispatch.rs` gained a third guard,
`reject_unrepresentable_layer_size`, in the same monomorphization-folded
shape as the two shm ones; it posts the protocol's own `invalid_size` and
refuses rather than clamping (a clamp leaves client and compositor
disagreeing about a size the client is about to draw at). In release the
assertion compiles out and the negative size instead reaches
`LayerMap::arrange`, which saturates its way to a nonsense geometry —
worth refusing either way. Found only because the tests are debug builds
and one of them sent `u32::MAX`; not by reading the handler.

**Tests**: 29 integration tests (`layer_shell/tests.rs`, all new — the
file does not exist on `main`) driving a real `wayland-client`
connection — binding `zwlr_layer_shell_v1`, `xdg_wm_base` and `wl_seat`
the way `waybar` and `fuzzel` do — through a real `State` with a real
headless backend, asserting on **read-back pixels**, on the core's own
arrangement, and on **what the client's own `wl_keyboard` was told**, not
on enum variants or compositor-side fields: the ordering bug above looks
correct at the type level, and "who has keyboard focus" is a claim about
what reached a client.

Geometry and input (16): no layer surfaces at all (today's behavior,
unchanged); top-layer-over-window and background-under-ring ordering;
exclusive zone moving windows (rect *and* pixels); two bars stacking on
one edge; `-1` reserving nothing; a surface that never commits reserving
nothing; the deliberate "reserved from the initial commit, not the first
buffer" behavior; destroy and client-disconnect both giving the space
back; `i32::MAX` geometry not overflowing anything; the `u32::MAX` size
refusal leaving the compositor serving; pointer hit-testing above and
below windows; clicking a bar not refocusing the window behind it (with
the control half — clicking the window *does* focus it); an output resize
re-arranging both bar and zone; and the pinned `xdg_popup` gap.

Keyboard interactivity (13): an `exclusive` overlay surface taking the
keyboard on map, the window getting its `leave`, and typed characters
arriving as `wl_keyboard.key`; a `none` bar never moving focus at all,
by mapping *or* by being clicked; a buffer-less surface not holding the
keyboard however loudly it asks; `exclusive` on `background` needing a
click; `on_demand` click-to-focus and all three ways back out (window,
bar, bare desktop); destroy, null-buffer unmap, and a later `none`
commit each returning it; front-most-wins across `overlay`/`top` with
fallback when the winner dies; a keybinding still firing (and the bound
key *not* reaching the client) while an exclusive surface holds the
keyboard; the zero-window case; and a client disconnecting mid-hold
leaving nothing behind.

15 new `flexwm-core` tests (5 in `geometry.rs`, 10 in
`world/tests/outputs.rs`; 60 total, up from 45 on `main`) plus
`Event::OutputUsableAreaChanged` (including degenerate and `i32`-extreme
rectangles) added to the randomized invariant test, which now also
asserts `usable ⊆ area` after every step.

Two negative controls, because a test that passes for the wrong reason is
worth less than no test:
- Moving the background-layer elements to the other side of the ring
  (i.e. back to what `space_render_elements` would have produced) fails
  the ordering test with "the focus ring over the wallpaper: wrong pixel
  at (9, 12)".
- Stubbing `layer_keyboard_focus` to `return None` fails **11 of the 13**
  keyboard tests; the two that still pass are exactly the two that assert
  nothing may change (`a_bar_that_wants_no_keyboard_never_takes_it`,
  `a_layer_surface_with_no_buffer_cannot_hold_the_keyboard`). Removing
  just the `last_acked.is_none()` mapped-ness check fails both
  buffer-less ones and nothing else.

**A harness bug worth recording, because it failed intermittently rather
than immediately**: the test client kept one `pending_ack` slot for all
its `xdg_surface`s, so with two windows mapped the compositor's
re-configure of the *first* could be acked against the second, and
Smithay rightly answered "must ack the initial configure before attaching
buffer" and killed the client — roughly one run in three. Configures are
now tracked per surface, and the fixture reports a dead client's own
error instead of a ten-second timeout (which is what hid it at first).
`Fixture::drop` also no longer joins the client thread while unwinding: a
compositor-side panic leaves that thread blocked on an answer that will
never come, which turned the `set_size` crash above into a hang with no
diagnosis.

**Hardware verification** — real `--tty` on the dev VM's `virtio-gpu` KMS
device at 1600x1000, driven by **real layer-shell clients**, `swaybg`
1.2.2 and `Waybar` 0.15.0 (both from `nixpkgs`, nothing written for the
occasion), against a **release** build of `a958633`, this branch's head.
Raw commands and output are in PR #22's description. The pointer is
parked at (1590, 990) before every capture, because the `--tty` cursor is
drawn at the sample point otherwise — an earlier run read the cursor's
own black outline pixel and briefly looked like a missing repaint.
Summary, with `flexwm msg screenshot` pixels:
- `swaybg -c '#00FF00'` on the background layer: `(1500,500)` reads
  `srgba(0,255,0)` where it read the compositor's own background
  `srgba(20,20,25)` a moment earlier, and `(9,500)` still reads the focus
  ring's `srgba(107,166,250)` — **the ring draws over the wallpaper on
  real hardware**, which is the ordering `space_render_elements` could
  not have produced. The windows do not move: `swaybg` reserves nothing.
- `waybar` on the top layer, height 30, default exclusive zone: both
  windows move from `y=12,h=976` to `y=42,h=946`, and back to
  `y=12,h=976` when it exits. Bar pixels read `srgba(255,0,0)` (its
  configured background) and `(800,35)` just below it reads the
  wallpaper.
- **Frame callbacks**: the clock region (600x30+500+0) differs by 47
  pixels between captures 4s apart, against exactly 0 for a capture
  compared with itself — the bar keeps redrawing rather than freezing on
  its first frame.
- **Pointer input reaches the layer surface**: with a `#clock:hover` rule
  in waybar's stylesheet, moving the pointer onto the clock turns that
  cell from `srgba(255,0,0)` to `srgba(0,0,255)`, and back when it
  leaves — `enter`, `motion` and `leave` all arrive.
- **Clicking the bar does not move window focus**: with window 2 focused,
  a click at (400,15) leaves both windows' `focused` flags unchanged;
  the control click at (400,500) moves focus to window 1.
- No `ERROR` or `WARN` from flexwm itself across the whole run (the two
  `smithay::backend::drm` lines every `--tty` run logs are filtered).
- **`wait-idle` now has a bar's redraw rate as its floor**, measured
  rather than assumed: with no layer surfaces, `--quiet-ms 200` settles
  in 203ms and `--quiet-ms 1500` in 1515ms; with waybar's clock ticking
  once a second, `--quiet-ms 200` still settles (204ms) and
  `--quiet-ms 1500` times out. A layer surface's commits count as screen
  activity like any other client's (`handlers.rs`'s `commit` sets
  `last_commit` for every surface), which is correct -- the screen really
  is changing -- but it is new agent-facing behavior, so it is in the
  README's layer-shell section too. Same shape as the caveat item 8
  recorded for an animated cursor.

**Benchmarked** (release builds, jiffies from `/proc/pid/stat`, the same
method items 5/8/13 used), because the render path changed shape:
`space_render_elements` was replaced, a per-frame layer-map lock added,
and a per-frame frame-callback pass over the layer list.
- **Idle is still exactly 0**: 0 jiffies over 10s before (`60348a5`) and
  after (`d4fc476`) with no layer surfaces, and 0 over 20s again at
  `a958633`.
- **150 corner-to-corner pointer jumps** (near-full-frame damage per
  jump), alternating binaries per rep so VM drift hits both arms:
  before `60348a5` 25/26/34 (mean 28.3), after `d4fc476` 24/21/25 (mean
  23.3), and 18/36/39 (mean 31.0) re-measured at `a958633`. **Read as
  overlapping ranges, not as a measured equivalence**: at n=3 per arm
  with an 18–39 spread inside a single arm, this rules out a large
  regression and nothing finer. What it does establish structurally, and
  the reason this arm exists at all, is that no per-frame allocation was
  added for a session with no layer surfaces — which is what those two
  binaries differ in.
- **A mapped bar costs nothing measurable on that workload**: same
  process, 150 jumps with waybar mapped 38/34/28, then with it gone
  18/36/39 — same n=3 caveat, same reading.
- **A bar's own redraws are close to free**: 3 jiffies over 20s with
  waybar's clock ticking once a second (1 jiffy in an earlier run), with
  the clock region provably changing over that window.
One measurement was thrown away rather than reported: the first "with a
bar" run read the wayland socket name out of a log line whose fields
`tracing` colorizes, got an empty string, and so measured a waybar that
had never connected. Every number above comes from a run that screenshots
the bar and prints the windows' `y` first.

Also at `a958633`: `cargo test` 240/240 for `flexwm` (15 layer-shell
tests at that commit) and 60/60 for `flexwm-core` (15 new, up from
`main`'s 45) on the dev VM, `cargo clippy --workspace --all-targets -D
warnings` and `cargo fmt --all --check` clean on both the VM and macOS,
`cargo check --workspace --all-targets` clean on macOS (the
cross-platform build), and `scripts/smoke-test.sh` green under
`--headless` against a release build of this branch (all 11 `ok:`
checks, exit 0).

Re-verified after the review-driven test additions (working tree at
`d816501` plus the `no_xdg_popup_is_configured_yet` / `Protocol error 1`
/ doc changes, committed as the branch head): `cargo test -p flexwm`
241/241, `cargo test -p flexwm-core` 60/60, `cargo clippy --workspace
--all-targets -- -D warnings` clean, `cargo fmt --all --check` clean,
all on the dev VM. `resize_output` — the one changed function whose only
production caller is `nested.rs`'s `apply_size` — was covered by running
the smoke test under the **`--nested`** backend too, not just
`--headless`: `WLR_BACKENDS=headless WLR_RENDERER=pixman
WLR_LIBINPUT_NO_DEVICES=1 cage -- env MODE=--nested
SHOT=/tmp/flexwm-smoke-nested.png ... scripts/smoke-test.sh` → all 11
`ok:` checks, exit 0, `/tmp/flexwm-smoke-nested.png` 1280x720 (cage's
mode, i.e. `resize_output` really did run and re-arrange against a size
that is not flexwm's built-in default) on the dev VM.

### Round two: keyboard interactivity — verification and benchmarks

Everything below was captured against **commit `9ddc295`** (working tree
clean), a **release** build (`/var/cargo-target/release/flexwm`,
3,551,816 bytes), on the dev VM's real `--tty` `virtio-gpu` KMS device at
1600x1000, driven by **real layer-shell clients**: `fuzzel` 1.14.1
(overlay layer, `keyboard_interactivity: exclusive` — a launcher, which
is the exact client class this work exists for), `swaybg` 1.2.2 and
`Waybar` 0.15.0, all from `nixpkgs`. Scripts and raw output are on the VM
at `/tmp/hw-evidence/` (`hw-out.txt`, `hw2-out.txt`, `hw3-out.txt`, and
the `hwshots*/` PNGs, which do not survive a VM reboot — everything
needed to reproduce them is in PR #22's "Reproducing" block instead); the
numbers here are copied from them verbatim. The pointer is parked at
(1590, 990) before every capture, for the same reason round one recorded.
**Cache key: this capture is no longer against `HEAD`.** It held through
the third review round (every commit after `9ddc295` up to `d5e673c` was
documentation), but that review then found a real focus bug, and the fix
for it changes `crates/`. See **Round four** at the end of this item for
what was re-verified against the new tree, what carries over, and why.

**Correctness, on hardware.**
- **Keystrokes reach the launcher, not the window behind it.** With
  `foot` focused and `fuzzel` mapped, `flexwm msg type "flexwmkeyboard"`
  leaves `> flexwmkeyboard` in fuzzel's prompt and **both terminals'
  prompts empty** (`hwshots/f-after-keybinding.png`). This is the bug
  that was shipping without it.
- **Keybindings still win, and the bound key is not leaked.** In the same
  screenshot, `flexwm msg key super+h` moved window focus (`focused: 2` →
  `focused: 1`, from `flexwm msg windows`) and fuzzel's prompt still
  reads exactly `flexwmkeyboard` — no trailing `h`. That is the VT-switch
  escape hatch working through the identical code path.
- **Keyboard returns when the launcher goes.** Typing
  `typedintoterminal` with only a bar mapped, then `intothelauncher` with
  fuzzel up, then `backtotheterminal` after `pkill fuzzel`, leaves the
  terminal reading `typedintoterminalbacktotheterminal` and nothing else
  (`hwshots2/r-bar-and-launcher.png`, `hwshots2/s-after-launcher.png`).
  The launcher's text never reached the terminal, and the terminal's text
  never reached the launcher.
- **A `none` bar is untouched.** `waybar` (clock module, red background)
  still moves the window from `y=12` to `y=42` when it maps and back to
  `y=12` when it exits, its own row reads `srgba(255,0,0)`, the wallpaper
  below reads `srgba(0,255,0)` and the focus ring at (9,500) still reads
  `srgba(107,166,250)` — i.e. the round-one render ordering and exclusive
  zone both still hold. With fuzzel mapped *over* the bar, the bar keeps
  its zone (`y=42`) and its pixels.
- **`swaybg` unchanged**: `(1500,500)` reads `srgba(0,255,0)` and
  `(9,500)` still reads the ring's `srgba(107,166,250)`.
- **No `ERROR` or `WARN` from flexwm itself** across any of the three
  runs (the `smithay::backend::drm` lines every `--tty` run logs are
  filtered).

**CPU at idle — the new focus path does not poll or spin.** Jiffies from
`/proc/<pid>/stat` (`utime+stime`), 20s windows, after `wait-idle`:

| state | jiffies / 20s |
| --- | --- |
| no layer surfaces | 0 |
| `swaybg` mapped, idle | 0 |
| `fuzzel` mapped and **holding exclusive keyboard focus**, idle | 0 |
| ticking bar (1Hz clock) only | 1 |
| ticking bar **+** focused launcher | 2 |
| focused launcher, no bar | 0 |

The bar's clock was *proved* to be redrawing rather than assumed:
`magick compare -metric AE -crop 120x30+0+0` between two captures 4s
apart reports **38.98** differing pixels, against **0** for a capture
compared with itself.

**CPU while typing — 200 characters, layer surface vs toplevel**,
3 reps alternating (`abcdefghij` × 20 via `flexwm msg type`, then
`wait-idle --quiet-ms 200`):

    rep1 layer_surface 3   toplevel 0
    rep2 layer_surface 1   toplevel 0
    rep3 layer_surface 0   toplevel 1

Both arms are within a few jiffies of zero for 400 key events, and the
ranges overlap. **Reported honestly rather than as "identical":** the
layer arm's mean is a jiffy or two higher, and the likely reason is not
the delivery path but the client — fuzzel redraws a 380x365 rounded box
per keystroke, where `foot` redraws a character cell. The compositor-side
work is the same `keyboard.input` → filter → forward either way.

**Latency.** `flexwm msg type "x"` → `wait-idle --quiet-ms 50`, wall
clock, 5 reps each:

    layer surface focused: 83 82 83 81 82 ms
    toplevel focused:      79 79 82 80 80 ms

A ~2ms difference on a measurement whose floor is the 50ms quiet window
plus two `flexwm msg` process spawns. Focus transfer itself is
synchronous inside the commit/destroy handler — there is no timer and
nothing deferred — so there is no separate "focus transfer latency" to
measure; what a user can feel is this number, and it doesn't move.

**No visible hitch across a focus transition.** Six screenshots taken
~50ms apart while the launcher is killed: frame 0 reads
`srgba(253,246,227)` (fuzzel's body) at (800,500) and frames 1–5 all read
`srgba(0,255,0)` (the wallpaper behind it). The launcher is gone by the
*first* frame after the kill; nothing is stale, nothing is half-drawn.
Polling a pixel for the same transition gave 135/171/170ms across three
reps, but that number is floored by how long a `flexwm msg screenshot`
takes (PNG-encoding 1600x1000), not by the compositor — the frame
sequence above is the better evidence.

**Memory.** `VmRSS` from `/proc/<pid>/status`: 23,336 kB at startup with
one window; 29,604 kB with `swaybg` mapped; 30,804 kB with `fuzzel` also
mapped. Across **15 map/unmap cycles** of the exclusive layer surface:

    before: 37252 kB
    cycle 1..4:  42108 kB
    cycle 5..15: 42112 kB      (+4 kB total across eleven cycles)
    after:  42112 kB, and 37272 kB by the end of the run

One step up on the first cycle (+4,856 kB), then flat to within 4 kB over
fourteen more — and the run ends back at 37,272 kB, so nothing accumulates
per cycle. The step's most likely mechanism, not instrumented further: a
layer-shell client's `wl_shm` pools are mmap'd into the compositor and
stay mapped until the dead surface is dropped, which for an implicit
teardown is the next `render()`'s `LayerMap::cleanup` — so "RSS right
after a cycle" includes whatever the last client left mapped. The thing
being tested here is whether that number *grows*, and it doesn't.

**Per-frame render cost is unchanged by holding focus.** 150
corner-to-corner pointer jumps (near-full-frame damage per jump), 3 reps
alternating: with a focused layer surface **36/35/47**, without
**36/39/36**. Same n=3 caveat as round one's numbers — overlapping
ranges, which rules out a large regression and nothing finer. Structurally
it should be zero: nothing was added to `render()` except one
`refresh_keyboard_focus()` inside the branch that already only runs when
`LayerMap::cleanup` dropped a dead surface.

**Fluidity, plainly.** Nothing in this feature is on a per-frame path,
idle cost stays at 0, and the only measurable difference anywhere is
~2ms of type-to-settle and a jiffy or two of typing CPU, both attributable
to the launcher's own redraw. It feels instant on the VM's software
renderer, and the numbers say why.

**What was not verified on hardware, and why.**
- **`on_demand` click-to-focus** (and `exclusive` degraded to `on_demand`
  on the background layer) has no convenient real client — `fuzzel` is
  `exclusive`, `waybar` and `swaybg` are `none`, and nothing in `nixpkgs`
  on this VM asks for `on_demand`. Both are covered by the integration
  tests, which drive a real `wayland-client` connection and assert on
  `wl_keyboard` events, but "a real `on_demand` client on real hardware"
  is an untested combination.
- **A real VT switch while a layer surface holds the keyboard.** The
  `super+h` check above is the closest proxy — same `key()` filter, same
  "intercepted before anything is forwarded" property — but
  `Ctrl+Alt+F<n>` itself was not pressed with `fuzzel` up. The structural
  argument is that `Bound::ChangeVt` and `Bound::Action` are both matched
  in the same filter before any forward, and that `session_event`'s
  pause/activate arms never touched keyboard focus for toplevels either
  (so nothing new is needed on the way back). That is an argument, not a
  measurement; it is listed here rather than claimed above.
- **A real layer-shell lock screen** (`gtklock`, `swaylock-effects`): the
  keyboard model is what one needs, and this PR is what stops one leaking
  keystrokes, but none was run — see the README's explicit note that a
  layer-shell locker here is a blanker you can type into, not a security
  boundary.

**Also at `9ddc295`**: `cargo test -p flexwm` **254/254** (241 → 254, the
13 new keyboard tests), `cargo test -p flexwm-core` **60/60**, `cargo
clippy --workspace --all-targets -- -D warnings` clean and `cargo fmt
--all --check` clean on the dev VM; `cargo fmt --all --check` and `cargo
check --workspace --all-targets` clean on macOS. `scripts/smoke-test.sh`
green against the release build under **both** backends — `--headless`
(11 `ok:` checks, exit 0) and `--nested` under `cage` (11 `ok:` checks,
exit 0, `/tmp/smoke-nested.png` 1280x720, i.e. `resize_output` ran
against a size that is not flexwm's default).

One methodological miss worth recording rather than hiding: the first
hardware run spawned `waybar` with no config, so it fell back to its
packaged default, whose `sway/*` modules mean it never maps a surface at
all — the run duly reported "the window did not move," which would have
read as a regression in the exclusive zone. Re-run with a two-module
config (`hw2.sh`), it maps and reserves exactly as before. The numbers
above are all from the re-run.

**Round four: a click outliving the state it was made against.** The
third review round reproduced a real focus bug (everything else it
checked held: VT-switch handling live, the negative-control tests exactly,
the fuzzel keystroke-leak capture independently). `clicked_layer` was
cleared by a click elsewhere, by `layer_destroyed` and by
`forget_dead_clicked_layer`'s liveness check — but by nothing when the
surface's own `layer_focus` became `Never`. So an `on_demand` surface
that was clicked, committed `keyboard_interactivity: none` (keyboard
correctly returned to the window) and then committed `on_demand` again
got the keyboard handed straight back with no new click, while the focus
ring, `set_activated` and `flexwm msg windows` all still named the
window. Same failure class as the screen-locker gap this item exists to
close, narrower in scope — and `none` ↔ `on_demand` is the normal
lifecycle for such a client, not an edge case. Fixed in `391529c` by
forgetting the click in `commit_layer_surface`, which is the only place
that transition can be observed (`layer_focus` reads committed state) and
which already had the `layer_focus` call, so the added cost is one
comparison against `None` on a `none` bar's redraw.

Four new tests (258 total, from 254). Red/green against the same tree
with only the fix reverted (`git stash push -- ...layer_shell.rs`):

    an_on_demand_surface_does_not_recapture_the_keyboard_after_committing_none ... FAILED
    an_on_demand_surface_does_not_recapture_the_keyboard_when_it_maps_again ... FAILED
      left: Some(Layer(0))   right: Some(Window(0))
    an_exclusive_surface_that_relaxes_to_on_demand_keeps_a_click ... ok
    an_exclusive_surface_that_relaxes_to_on_demand_unclicked_gives_the_keyboard_back ... ok

i.e. both regression tests really do reproduce the bug, and the
`Exclusive` → `OnDemand` path the fix must not disturb passes on *both*
sides of it. The unmap/re-map variant needed a new `Step::RemapLayer`:
`UnmapLayer` could only ever be terminal before, because Smithay's
`got_unmapped` resets the cached state to `Default`, so a client has to
re-send its size, anchors *and* layer before committing again, and the
re-map must wait for a configure by count — the unmap's own commit
already provokes one, measured at 100x100 for a 60x60 launcher.

**Cache key, and what was re-run.** Everything above this section was
captured at `9ddc295`; `391529c` changes `crates/`, so that key no longer
matches for anything the fix touches. Per the review's own scoping — a
`clicked_layer` clear cannot plausibly move idle CPU or RSS — the full
benchmark suite was *not* re-captured; the two checks that could
regress were, on the same real `--tty` seat at 1600x1000, release build
of **`391529c`** (clean tree), script and raw output at
`/tmp/hw-evidence/round4.sh` and `round4-out.txt`, captures in
`/tmp/hw-evidence/hwshots4/`:

- **The escape hatch still works and still doesn't leak.** Two `foot`
  windows, `fuzzel` 1.14.1 mapped and holding the keyboard.
  `flexwm msg type "flexwmkeyboard"` changed **1329.08** pixels, and
  `-trim` on the difference mask puts *all* of them inside
  `326x151+637+324` — a patch in the middle of a 1600x1000 screen, which
  is where fuzzel is drawn (its body pixel at (800,500) reads
  `srgba(253,246,227,1)`). Neither terminal's text area is in that patch,
  so none of it reached them. Two captures with nothing in between differ
  by **0** pixels over that region, which is what makes the next number
  mean something: after `flexwm msg key super+h` the same region differs
  by **0** — no `h` reached fuzzel — while window focus moved
  (`{1: false, 2: true}` → `{1: true, 2: false}`) and the whole frame
  differs by **4606.72**, i.e. the ring really moved. The region is
  *derived* from the typing diff rather than guessed at, so it cannot be
  a crop that happens to miss the text.
- **No RSS growth across 15 fuzzel map/unmap cycles.** `VmRSS` 35,652 kB
  before; 35,660 / 35,840 / 35,844 for cycles 1–3, then 35,844 flat
  through cycle 10, 35,848 from cycle 11 through 15; 35,848 kB after.
  **+196 kB total, +4 kB across the last thirteen cycles** — the same
  shape round three recorded (a small step early, then flat), on a run
  that happens to start lower.
- **No `ERROR`/`WARN` from flexwm itself** (the `smithay::backend::drm`
  master line every `--tty` run logs is filtered).

Not re-run, deliberately: idle jiffies, the keystroke-burst comparison,
the pointer-jump render timings and the transition screenshots. Nothing
in the fix is on a per-frame path — it is one comparison inside a commit
handler that already read the same value — and review's own assessment
was that a full hardware re-capture is not warranted for it. The fix's
own behaviour also cannot be hardware-tested here for the reason already
listed above: no `on_demand` layer-shell client exists on this VM.

**Also at `391529c`**: `cargo test -p flexwm` **258/258**, `cargo test -p
flexwm-core` **60/60**, `cargo clippy --workspace --all-targets -- -D
warnings` clean and `cargo fmt --all --check` clean on the dev VM;
`cargo fmt --all --check` and `cargo check --workspace --all-targets`
clean on macOS. `scripts/smoke-test.sh` green against the release build
under both backends — `--headless` (11 `ok:`, exit 0) and `--nested`
under `cage` (11 `ok:`, exit 0, `/tmp/smoke-nested-round4.png` 1280x720).
