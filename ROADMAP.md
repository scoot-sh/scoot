# flexwm roadmap

The ordered list of milestones, worked through with the cycle in
`CLAUDE.md`. Update this file in the same PR that lands each item — it's the
authoritative, git-tracked history of what shipped, what was found in
review, and why.

## Order

1. ~~Nested backend~~ — DONE, merged to `main` at `494048b`. Independent
   review found 2 real low-probability bugs (resize-failure wedge,
   skipped-frame-never-retried) + 1 unneeded `unsafe impl Send`; all fixed
   and re-verified (`--headless`/`--nested` smoke test, 19 tests, clippy
   clean) before merge.

2. ~~Keybindings layer~~ — DONE, merged to `main` at `f3723b3`, PR #4. Vim
   motions (h/j/k/l) + Super, intercepted at `input::key`'s filter closure
   (the single choke point shared by IPC/nested/future-tty input). Matches
   unshifted level-0 keysym + tracked modifiers (not case-folded shifted
   symbols) — deliberate deviation from initial spec, matches niri/sway
   convention. Independent review found 2 issues, both fixed before merge:
   (a) `press()` could leak a stuck modifier if the combo's main key resolved
   by name but wasn't on the actual keymap — fixed by resolving the main
   keycode before pressing any modifiers; (b) documented an invariant on
   `suppressed_keys` (only `key()` may mutate it) against a desync risk
   relevant once the tty backend's libinput device-teardown exists.
   `crates/flexwm/src/compositor/keybindings.rs` holds the pure, unit-tested
   lookup table — the seam a config-file loader parses `flexwm_ipc::KeyCombo`
   strings into.

3. ~~Real tty/DRM backend~~ — DONE, merged to `main` at `0a90dc9`, PR #5.
   `--tty`: libseat session + raw `DrmSurface`/dumb-buffer scanout (no GBM —
   this Smithay rev has no `Bind<DumbBuffer>` for pixman, so presenting is a
   memcpy, confirmed necessary not just simplest) + libinput. Mirrors
   `nested::Host`'s shape (`Tty::present` same byte-slice signature). VT
   switching added as `keybindings::Bound::ChangeVt` (kept out of
   `flexwm_core::Action` — session concern, not layout, keeps the core
   platform-independent). Two independent review rounds; second found 3 real
   issues, all fixed: (a) `reactivate()` used to skip `libinput.resume()` on
   a failed `drm.activate`, which could strand the keyboard dead alongside a
   dead display with no recovery path — fixed so every recovery step runs
   independently; (b) `DrmEvent::Error` didn't free the pending buffer slot
   the way `VBlank` did, leaking it forever after repeated occurrences —
   fixed via a shared `flip_settled()`; (c) `build.rs`'s `pkg-config`
   build-dependency was wrongly gated behind `cfg(target_os = "linux")`,
   which broke `cargo check`/`cargo build` on the Mac dev host (build-deps
   compile for the *host*, not the target) — fixed by ungating it. Verified
   on the dev VM's real `virtio-gpu` KMS device: real scanout, VT-switch
   round trip (pause/reactivate + forced full modeset on the way back), idle
   CPU ~0, a real `/dev/uinput`-injected keystroke proven to reach a client.
   The "real scanout" and VT-switch claims here were the ones later called
   into question by Smithay's `unprivileged mode` warning; both were
   re-confirmed directly on 2026-09-13 (the CRTC's plane really does scan out
   a flexwm-allocated framebuffer, and master really is dropped and
   reacquired across a VT switch) — see the resolved DRM-master entry in the
   Backlog.
   Out of scope, stated: cursor rendering, DRM hotplug, multi-GPU/output,
   DPMS, output scale, key-repeat.

4. Window decorations and a config file for "more visually pleasing options."
   Split into two PRs. **4a config file — DONE, merged to `main` at
   `3b62292`, PR #6.** `crates/flexwm/src/compositor/config.rs`: TOML at
   `$XDG_CONFIG_HOME/flexwm/config.toml` (falls back to `~/.config/flexwm/`),
   `--config PATH` flag; `[layout]` + `[binds]` sections. serde stays out of
   `flexwm_core` (conversion by hand in the `flexwm` crate). Failure
   semantics are the safety-critical part: explicit `--config` missing = hard
   error; default path missing = silent defaults; malformed/typo'd config =
   logs and falls back to full defaults, *never* blocks startup (a
   compositor that won't boot over a typo is a lockout with no recovery on
   real hardware). Bind collisions are detected and the whole group skipped
   rather than an arbitrary "last wins." `--tty`'s Ctrl+Alt+Fn VT-switch
   bindings always override a colliding config bind, applied after the
   config loads. 51 tests, verified via `scripts/smoke-test.sh` under all
   three backend modes including real `--tty` hardware.
   **4b decorations — DONE, merged to `main` at `21b9c3e`, PR #7.**
   `crates/flexwm/src/compositor/decorations.rs`: niri-style focus ring
   (width clamped to at most half the gap at load time) drawn in the
   layout's own gap + a background color (`render_output`'s `clear_color`,
   not an element — bottom-most by construction) + `zxdg_decoration_manager_v1`
   answering `ServerSide` when `prefer_no_csd` (default true).
   `headless.rs::render()` moved off `space::render_output` to
   `space_render_elements`+`OutputDamageTracker::render_output` directly so
   windows draw on top of the ring (element order is back-to-front,
   `.iter().rev()`). Ring buffers are persistent per-window (`SolidColorBuffer`,
   stable `Id`, updated not rebuilt). Verified via real pixel-sampled IPC
   screenshots across all three backends including real `--tty` hardware.
   `imagemagick` added to `vm/configuration.nix` for pixel-reading.

5. ~~Cursor rendering for `--tty`~~ — DONE, merged to `main` at `546534b`,
   PR #8. `crates/flexwm/src/compositor/cursor.rs`: a persistent,
   procedurally-generated arrow bitmap (no copied cursor-theme asset —
   deliberately license-clean per `CLAUDE.md`) rendered front-most, gated to
   `--tty` only. Damage-scoped presentation: `tty/buffers.rs` tracks each
   dumb buffer's own age (the two slots don't age in lockstep) so
   `tty::present()`'s memcpy scopes to the damaged bounding box instead of
   the full frame on every mouse-motion-triggered render. Independent review
   found two real issues, both fixed before commit: (a) an off-by-one in the
   age calculation (`generation - last_written + 1`, not `generation -
   last_written` — the render being peeked-for hasn't happened yet) that
   would have left stale pixels on the scanout buffer under slot alternation,
   invisible to every existing check since they all read the pixman
   intermediate, never the actual DRM buffer this lived in; (b)
   `write_region`'s pitch/offset copy math had zero test coverage, closed by
   extracting pure `age`/`copy_region_rows` helpers with regression tests.
   Both fixes are unit/source-verified against the pinned Smithay damage
   tracker, not empirically screenshot-verified — screenshots structurally
   can't observe the scanout buffer. Benchmarked via jiffies-delta: idle
   unchanged at 0, ~7 jiffies/150 small local moves vs ~42/150 near-full-frame
   corner jumps (the latter ≈ pre-fix cost) — confirms the scoping fix is
   real. `CursorImageStatus::Named`/`::Surface` both drew the same fallback
   shape; `::Surface` was fixed in item 8. `::Named` still draws the fallback
   and always will (there is no client buffer behind a name) — item 13 made
   that shape's size and color configurable; a *per-name* shape is still
   Backlog (b) and needs a licensed asset source first.

   **5a. Merge authority history.** `gh pr merge` was denied by the
   auto-mode classifier ("Merge Without Review") when attempted autonomously
   on PR #8 — a harness-level gate. The user approved merging PR #8 in chat;
   for PR #9 that approval didn't carry forward and a fresh check-in was
   asked for and given. After that, the user made it standing policy (see
   `CLAUDE.md`): once `flexwm-reviewer` has reported back and the
   coordinating session has actually weighed the diagnosis, merge without
   asking again — first exercised merging PR #9 and #10 together.

   **5b. ~~VT-switch-back `EPERM`~~ — DONE**, PR #9. Root cause confirmed by
   real reproduction on `--tty` hardware: `State::change_vt` (`tty/mod.rs`)
   had no gate on session-paused state. Real hardware input never triggers
   this (`SessionEvent::PauseSession` correctly calls `libinput.suspend()`
   first, pre-existing item 3) — but the IPC `key` request is a second,
   independent input path that bypasses `libinput` entirely, so switching
   away for real then sending the switch-back combo over IPC while genuinely
   paused reliably hit `libseat`'s `EPERM` on `VT_ACTIVATE`.

   First fix attempt gated on the existing `Tty::active` field and was
   **wrong, caught by the coordinating session before it reached a separate
   reviewer** — the incident `CLAUDE.md`'s "independent review is mandatory"
   section is named after: `active` means "DRM master held," not "session
   active" — it also goes `false` inside `reactivate()` when `drm.activate`
   itself fails, a state item 3's own review made deliberately recoverable
   ("dead DRM device, working keyboard, retry the VT switch"). Gating
   `change_vt` on `active` silently broke that retry path. Corrected fix: a
   separate `Tty::session_paused` field, set `true` only in the
   `PauseSession` arm and cleared `false` in the `ActivateSession` arm
   *unconditionally, before* `reactivate()` runs — so `session_paused` can
   never depend on whether that call's own `drm.activate` succeeds.
   `change_vt` gates on `session_paused`, not `active`. Re-verified on real
   hardware both ways: (1) the original IPC-while-paused repro still gets a
   clean skip, no `EPERM`; (2) after a real reactivation cycle, `change_vt`
   works normally again for a fresh switch-away/back over IPC. 93/93 tests
   pass, clippy/fmt clean, cross-platform build clean on macOS. Those
   pause/reactivate cycles were real in the strongest sense, not just
   libseat-event-level: confirmed 2026-09-13 that a `--tty` session over SSH
   holds real DRM master and really does lose and reacquire it across a VT
   switch (see the resolved DRM-master entry in the Backlog).

   `flexwm-reviewer`'s first formal pass (PR #9): **no blocking findings** —
   independently re-verified the whole test/clippy/fmt trio, traced every
   read/write site of both `active` and `session_paused` and confirmed they
   never conflate (the two write-site sets are structurally disjoint, not
   just correct by current ordering), checked the fix's premise against the
   pinned Smithay source (`Event`'s two variants are exhaustive, no third
   path can move session state behind flexwm's back) and against the actual
   `seatd` binary's own refusal strings on the dev VM. Two small
   observations addressed before merge: the skip's log level was raised
   `debug!` → `info!` (an explicitly requested action being silently
   discarded needs a trace at the default log level, not just "no spurious
   error"), and `session_paused: false` at `init` — which asserts a state
   rather than observing an event, the same shape as the mistake that made
   `active` wrong the first time — got a doc comment explaining why it's
   actually safe (`init`'s own `session.open()` call already fails first if
   the session isn't active). Two more were logged as backlog, not fixed
   here: an existing pre-PR `warn!` on "already on that VT" that predates
   this change, and the IPC-VT-switch note below, sharpened by this review.

   General note, not a fix, sharpened by `flexwm-reviewer`'s pass: every IPC
   input request (`pointer_move`/`pointer_button`/`scroll`/`key`/`type`)
   bypasses `libinput`'s suspend the same way `change_vt` did — `change_vt`
   was the only one that turned into a hard `libseat` error, so it's the
   only one fixed here. The concrete, sharper version of why this matters
   for `flexwm-vision`'s "an agent doing computer-use tasks in a VM is a
   first-class client" goal: an agent that sends a VT-switch-away over IPC,
   then a VT-switch-back over IPC, gets a clean skip instead of an error on
   the second call — but the compositor is still off-screen, and IPC is the
   agent's only input channel, so it cannot un-pause itself. An
   IPC-initiated VT switch away is currently a one-way door for anything
   whose only input is IPC. See the backlog entry below.

   Process note: a `pkill -f` self-match gotcha came up during this item's
   testing — a bare pattern that literally contains the invoking shell's own
   command line kills the calling shell too; worked around with the bracket
   trick `pkill -f 'flexw[m]'`.

   **5c. ~~IPC-initiated VT switch is a one-way door~~ — DONE**, PR #11.
   Picked up from the Backlog, sharpened by `flexwm-reviewer`'s PR #9 pass
   (see 5b above). Resolution chosen deliberately over gating/rejecting: 5b's
   own hardware bug-bash relies on IPC being able to trigger `ChangeVt`, so
   blocking it there would break that test path. Instead the *reply itself*
   now tells an agent whose only input/output is IPC what just happened,
   since IPC has no route to a log line the way a human at the console does.

   `Tty::change_vt` (`tty/mod.rs`) now returns a new `VtSwitchOutcome`
   (`Requested`/`Ignored`/`IgnoredPaused`/`Failed`) instead of `()`, threaded
   up through `input::key`'s new `KeyOutcome { intercepted, vt_switch }`
   (replacing its old bare `bool`) and `press()`'s new
   `Result<Option<VtSwitchOutcome>, String>` (replacing `Result<(), String>`,
   accumulated with `Option::or` across every press in the combo, not just
   read from the main key, so it stays correct even if a future binding
   changed which key can carry one) to `ipc.rs`'s `Request::Key` handler —
   `press()`'s one caller. A new `Response::Warning { message }` variant
   fires for `Requested` and `IgnoredPaused`; `Failed` gets `Response::error`
   (libseat itself refused the request — not "success with a side effect,"
   an actual failure); plain `Ignored` (no `--tty` backend at all) and no VT
   binding matched stay a plain `Ok`. `PROTOCOL_VERSION` bumped 1 → 2:
   `Response` is internally tagged, and this project's own
   `unknown_request_types_are_rejected` test proves an unrecognized tag is a
   hard decode error for an older client, not something it can shrug off —
   so this addition does break old clients' decoding, contrary to the first
   pass's assumption that it was purely additive. `flexwm msg` prints a
   `Warning`'s JSON to stdout the same way every other response does (so
   `flexwm msg key ... | jq .` still works and reflects what happened) plus
   a human-readable line on stderr; exits `0` either way (a warning isn't a
   failure, `Response::error` still is).

   `Requested` means "libseat accepted the request," not "a switch is
   guaranteed" — found empirically on the dev VM, not assumed: requesting
   the VT this session is *already showing* also returns `Ok(())`, with
   `seatd`'s own log reading "Could not set next session: requested session
   is already active" and no pause at all. `libseat_switch_session`'s own C
   doc says the same thing plainly ("does not imply that a switch will
   occur"). The warning's wording accounts for this directly ("requested...
   if it takes effect...") rather than asserting a pause that may not
   happen — deliberately still warning on this no-op case rather than
   trying to suppress it (that would need tracking which VT this session
   currently occupies, which nothing here does today — see the Backlog entry
   below).

   **`flexwm-reviewer`'s first pass on this item (against the pre-fix
   version) found real issues, all fixed before this write-up**: (1) the
   actual ticket gap — the switch-*back*-while-paused retry over IPC still
   replied with a bare `Ok`, with the "why nothing happened" explanation
   only in the compositor log, i.e. exactly the ambiguity 5b's own problem
   statement calls out. Root cause: the original single `Ignored` variant
   collapsed "no `--tty` backend" and "session is paused" into one case,
   so `ipc.rs` couldn't warn on the second without also (wrongly) warning
   on the first. Fixed by splitting `Ignored`/`IgnoredPaused` as described
   above. (2) `Failed` silently read as success (`Response::Ok`) — fixed to
   `Response::error`. (3) the warning text claimed "only real hardware
   input... can reactivate it," which the review's own hardware repro
   disproved (`chvt 1` from an ordinary ssh shell reactivated it, no
   physical keyboard involved) — reworded to "a VT switch from outside this
   compositor — a physical Ctrl+Alt+Fn, or `chvt N` from any shell on this
   machine." (4) the `PROTOCOL_VERSION` bump above, confirmed necessary by
   the project's own decode-error test rather than left unbumped on a
   "probably fine" assumption. (5) two doc-comment inaccuracies: `KeyOutcome`
   claimed `keyboard.input`'s `None` case meant "no active keyboard focus
   target," but the pinned Smithay rev's `input_from_source` (`src/input/
   keyboard/mod.rs`) shows `None` actually comes from either a keycode
   already held by another input source, or the filter returning `Forward`
   — focus isn't involved; and `VtSwitchOutcome`/`press()`'s docs said a
   switch-away makes the compositor "unreachable over IPC," which this same
   PR's own hardware evidence disproves (`flexwm msg key`/`flexwm msg
   windows` both work fine while paused) — narrowed to what's actually true:
   the one channel that could switch the session *back* is what's lost, not
   IPC reachability generally. (6) `press()` read `vt_switch` from only the
   main key's press, correct today only because nothing but the hardcoded
   VT bindings constructs a `ChangeVt` — made robust instead of
   documentation-dependent via the `Option::or` accumulation described
   above. Re-verified in full afterward: 93/93 tests, clippy/fmt clean on
   the dev VM guest, cross-platform build clean on macOS,
   `scripts/smoke-test.sh` full pass under `--headless` (confirming
   `Ok(None)` — no `ChangeVt` binding outside `--tty` — stays a plain `Ok`
   with no spurious warning where this doesn't apply), and all five
   `--tty` hardware scenarios re-run on the dev VM's real `seatd`-backed
   session, including the previously-missing one: retrying the switch-back
   combo over IPC while genuinely paused now gets `Response::Warning`
   ("ignored: this session is already paused..."), not a bare `Ok`, and
   still no `EPERM` (5b's fix unregressed). Exact commands and raw
   command/output blocks for every scenario are recorded in PR #11's
   description rather than narrated here. `Failed → Response::error` is
   code-traced, not hardware-exercised — there's no cheap way to force a
   real libseat error on this VM, same caveat 5b made about forcing
   `drm.activate` to fail. No unit test constructs a live `Tty`/`State` to
   exercise any `VtSwitchOutcome` variant directly — same reasoning as 5b:
   no existing fixture for that, hardware bug-bash plus code tracing is this
   project's established way of verifying this class of session-state
   correctness.

6. A real GPU rendering pipeline, added at the end after everything above is
   stable — an actual goal, not just a "don't foreclose it" constraint.
   Likely a GLES/Vulkan Smithay renderer as an alternative to pixman,
   selected per-backend (e.g. tty backend prefers GPU when a real DRM/GBM
   device supports it, headless/nested/webtop keep pixman) rather than
   replacing CPU rendering outright, since GPU-free operation for
   webtop/no-GPU boxes stays a hard requirement. This is exactly why the
   render-target/presentation split from item 1 onward matters: it's the
   seam a GPU renderer slots into.

7. ~~`wl_shm_pool.resize(0)` aborted the whole compositor~~ — DONE, PR #12.
   Out of the order above on purpose: a live crash-DoS jumps the queue, so
   this landed while item 6 hasn't started. Found by the security audit
   recorded in the Backlog below (2026-09-12, against `main` at `2b92928`),
   its one CRITICAL finding.

   The bug is upstream, in the pinned Smithay rev (`0ff00983`,
   `src/wayland/shm/handlers.rs:185-190`): the `wl_shm_pool.resize` handler
   posts a protocol error for `size <= 0` but is **missing the `return`**
   after it, so `size == 0` falls through into
   `NonZeroUsize::try_from(0).unwrap()` and panics. `[profile.release]` sets
   `panic = "abort"`, so this was not the contained single-client protocol
   error `state.rs`'s dispatch-site comment assumes — it aborted the process
   and every connected client's session with it. Trigger: any client,
   `wl_shm.create_pool(fd, 1)` then `wl_shm_pool.resize(0)`. No crafted fd
   contents, no privilege. Still present on Smithay `master` as of this
   date, so there was no version bump to wait for.

   Fixed inside flexwm, with no fork of Smithay — the fallback option, which
   would have needed an externally-hosted one-line fork behind a Cargo
   `[patch]`, turned out not to be necessary. What forced the shape:
   (a) this rev has no `delegate_shm!` to partially override —
   `delegate_dispatch2!` generates a single *blanket* `Dispatch` impl over
   every interface, and without specialization any per-interface impl
   overlaps it (E0119), so the blanket impl is the only seam flexwm owns;
   (b) the valid-size path can't be reimplemented locally — `ShmPoolUserData`'s
   only field is private and `shm::pool::Pool` isn't exported. So
   `compositor/dispatch.rs` now holds a hand-written copy of what the macro
   expanded to, plus one guard that rejects `size <= 0` before Smithay sees
   it; `size > 0` still goes to Smithay untouched.

   The guard is on the per-request path, so it's written to fold away:
   `TypeId::of` is a `const fn`, so after monomorphization both sides of its
   first comparison are compile-time constants and the body disappears for
   every interface other than `wl_shm_pool`. Measured, not assumed — 1M
   `wl_surface.damage` requests, compositor CPU in jiffies, 5 reps:
   before 49/35/32/36/37, after 35/35/34/32/34 (~1.6-1.8M req/s either way).
   Overlapping ranges, no measurable cost.

   One documented claim was **wrong until hardware disproved it**: the first
   draft said negative sizes would change error message from "mremap failed"
   to "invalid wl_shm_pool size". They don't — upstream already posted
   exactly that error before falling through, and only a client's *first*
   protocol error is ever delivered, so negative sizes are byte-identical
   before and after (verified on release builds both ways). What the guard
   does drop for them is a pointless trip through `Pool::resize`, whose
   `MemMap::remap` unmaps the existing mapping *before* discovering the new
   one can't be made.

   Verified on the dev VM against real `--headless` **release** binaries
   (`panic = "abort"` is what makes the difference visible): before, the
   compositor died with `Aborted (core dumped)`, exit 134/SIGABRT, and a
   second client got "Connection refused"; after, it stays up, the offender
   alone is disconnected, and a fresh client still sees all 9 globals. Nine
   adversarial cases (0, -1, `i32::MIN`, `i32::MAX`, a legal no-op grow, the
   ordinary grow, `create_pool` with 0 and -1, and a repeat from a second
   connection) all leave it alive. Two regression tests drive a real
   `wayland-client` connection through a real `State`'s real dispatch and
   assert an innocent second client is still served; the zero case fails
   against the unfixed tree with the upstream panic. 95 tests, clippy/fmt
   clean, `scripts/smoke-test.sh` green (its `foot` windows exercise the
   ordinary shm buffer path through the new dispatch), macOS cross-build
   clean.

   Delete `dispatch.rs`'s guard and go back to
   `smithay::delegate_dispatch2!(State)` once a Smithay bump carries the
   missing `return`.

8. ~~Client-supplied cursor images (`CursorImageStatus::Surface`)~~ — piece
   (a) of the Backlog's "Custom/client cursor support" entry. Piece (b) (a
   config-level override for the *fallback* shape) was untouched here and
   landed later as item 13, for size and color only — a theme *name* is still
   open, for the license reason that entry gives. Like item 7, this landed
   ahead of item 6: it's small, self-contained, and item 5 shipped knowing
   it was wrong.

   Before this, `cursor.rs` drew its procedural 16x16 triangle for every
   status except `Hidden`, so a client that handed the compositor a real
   `wl_surface` full of cursor pixels (an I-beam, a resize arrow, a spinner)
   got the triangle instead. Now `Cursor::element` renders that surface's
   subsurface tree via
   `render_elements_from_surface_tree`, at the hotspot the client set.
   `Named` still draws the fallback and always will from here: there is no
   client buffer to draw for it, only a theme name.

   Four things the shape of this fix turned on, all verified against the
   pinned Smithay rev (`0ff00983`) rather than assumed:

   - **The hotspot isn't on the enum at this rev.** `CursorImageStatus` is
     `Surface(WlSurface)` (`src/input/pointer/cursor_image.rs:42`) — the
     hotspot lives in the surface's own `data_map` as a
     `CursorImageSurfaceData`, written by Smithay's `wl_pointer.set_cursor`
     handler. It is therefore **re-read every frame, never cached**: a
     client may call `set_cursor` again with the *same* surface and a
     different hotspot, producing a `CursorImageStatus` that compares equal
     to the previous one, so "the status didn't change" does not imply "the
     hotspot didn't change." Covered by
     `a_new_hotspot_on_the_same_surface_moves_the_cursor`.
   - **Two element types, one return.** The fallback needs
     `R: ImportMem` + `MemoryRenderBufferRenderElement`; a surface needs
     `R: ImportAll` + `WaylandSurfaceRenderElement`, and a tree can produce
     more than one. `cursor.rs` now defines a `CursorElement<R>`
     (`render_elements!`, still renderer-generic — this module and
     `decorations.rs` deliberately don't hard-code `PixmanRenderer`) and
     `element()` returns a `Vec` of it: empty for hidden/no-content, one for
     the fallback, N for a tree. `headless.rs`'s `Elements::Cursor` variant
     wraps that instead of the bare memory element.
   - **Frame callbacks, or animated cursors freeze.** A cursor surface is
     never in `self.space`, so `render()`'s per-window `send_frame` loop
     could never reach it — and a well-behaved client attaches one frame,
     asks for a callback, and waits. `render()` now also calls
     `send_frames_surface_tree` for the cursor surface, **only** on frames
     that actually drew it (`Cursor::surface()` and `Cursor::element()` both
     go through one private `live_surface()`, so "was it drawn" and "does it
     get a callback" cannot drift apart).
   - **Nothing upstream drops a destroyed cursor surface.** Confirmed by
     reading `wayland/seat/pointer.rs`: its destruction handling covers the
     `WlPointer` object, not the cursor surface, so a client that destroys
     its cursor surface without setting a replacement leaves
     `CursorImageStatus::Surface(<dead>)` in place indefinitely. Two
     independent defenses, both exercised: `CompositorHandler::destroyed`
     (newly overridden) calls `Cursor::forget_surface`, which resets to the
     default named shape and requests a redraw under `--tty`; and
     `live_surface()` independently refuses a non-`alive()` surface, so the
     render path stays safe even if `dispatch.rs`'s hand-written
     `Dispatch::destroyed` forwarding (see item 7's maintenance hazard) ever
     regresses. Falling back to the built-in shape rather than to `Hidden`
     is deliberate: the pointer still exists and is still being moved.

   `with_states` on a destroyed surface is safe, not lucky: the `WlSurface`
   proxy owns an `Arc` of its object data (`wayland-scanner-0.31.11`'s
   `server_gen.rs:139-141`), so `Resource::data()` keeps working after
   destruction — which is also why Smithay's own `IsAlive for WlSurface` can
   unwrap it. The hotspot lookup deliberately does no rendering inside its
   `with_states` closure: that guard is a plain non-reentrant `Mutex` and
   walking the tree to render re-locks the same one.

   Seven new integration-style tests in `cursor/tests.rs` drive a real
   `wayland-client` connection through a real `State` (the harness
   `dispatch/tests.rs` introduced, extended with a step-at-a-time script
   channel so the client and compositor halves can interleave), then render
   with a real `PixmanRenderer` and assert on **read-back pixels**, not on
   which enum variant came out — a variant assertion would pass on a
   wrongly-positioned or wrongly-imported element. Covered: the happy path
   and hotspot placement, a re-set hotspot on the same surface, a cursor
   surface with no buffer yet (draws nothing — deliberately *not* the
   fallback, which would flash a wrong shape between `set_cursor` and the
   client's first commit), destroy-while-active, 8 rounds of
   hidden/visible alternation, a cursor surface with a subsurface (two
   elements), and an `i32::MIN`/`i32::MAX` hotspot — the one
   client-controlled integer that reaches element geometry, which
   `wl_pointer.set_cursor` takes raw and unbounded. That one places a real
   element at the saturated coordinate and asserts nothing is drawn and
   nothing panics (a debug build is where the damage tracker's own
   `loc + size` would blow up), then that an ordinary hotspot still works
   afterwards. Every test was confirmed non-vacuous against at least one of
   three negative controls (`live_surface` forced to `None`, `surface_hotspot`
   forced to `(0, 0)`, `forget_surface` stubbed to a no-op), each disabling a
   different piece of production code and watching the relevant tests fail
   with the specific wrong value the old fallback behavior would produce —
   see the PR for the raw output.

   **Hardware verification** (dev VM, real `--tty` on its `virtio-gpu` KMS
   device at 1600x1000, all against `03cd51c` — no production code changed
   since; a later commit added one more test and this paragraph). Driven
   by a throwaway raw-protocol client (built in `/tmp` on the VM, nothing
   committed, no image asset involved — the pixels come from the client over
   the wire) that maps a toplevel and hands over a flat-coloured cursor
   surface of a chosen size and hotspot. Exact commands and raw
   pixel/jiffies output are in the PR description. Summary:
   a 24x24 magenta cursor with hotspot `(4,6)` lands pixel-exact around the
   pointer; a 128x128 one does too, including clipped against the output's
   top-left corner; leaving the client's surface falls back to the built-in
   shape (Smithay resets to `default_named()` on focus leave,
   `input/pointer/mod.rs:823`); an `--animate` client's frame-callback
   counter sat at **0 for 187s** while its cursor wasn't presented, then
   climbed at ~46/s the moment the pointer entered, and **froze again**
   (1156 → 1156 over 3s) when the pointer left — the callback gating works
   in both directions; screenshots caught both animation colours. Destroying
   the cursor surface while active, and `kill -9` on the client while
   active, both leave the compositor up and drawing the fallback, with no
   panic or error in its log.

   **Benchmarked** (same jiffies-delta method as item 5), because the render
   path changed shape: the fallback now goes through an enum wrapper and
   `element()` returns a `Vec`. 12 interleaved reps per side of the
   expensive case (150 corner-to-corner pointer jumps, near-full-frame
   damage), alternating the pre-change `a1693e4` binary and this one so VM
   drift hits both: **before mean 53.58 (42–59), after mean 52.58 (44–62)**
   — no measurable difference. A static client cursor parked under the
   pointer costs **5 jiffies over 10s**, i.e. no render-loop spin was
   introduced (a frame callback only goes to a client that asked for one).
   An animated one costs 80 jiffies over 10s while it animates, which is
   just what drawing an animation at frame rate costs; `wait-idle` never
   settles while one is running, expected and worth knowing.

9. ~~IPC hardening: socket permissions, peer credentials, a bounded request
   line, screenshot rate limiting~~ — DONE, PR #14. Four Backlog findings from
   the 2026-09-12 security audit, landed together because they touch the same
   two files. Ahead of item 6 for the same reason items 7 and 8 were: small,
   self-contained, and already written up.

   Context that shaped the scope: sway's own `ipc-server.c` (read, not assumed)
   does no `chmod` and no peer check either — it relies entirely on
   `$XDG_RUNTIME_DIR` being `0700`, as do i3, niri and Hyprland. So flexwm was
   not behind anyone architecturally. What makes the asymmetry worth closing
   anyway is that flexwm's IPC injects arbitrary input and returns
   screenshots, where theirs mostly reads state and runs layout commands — so
   the blast radius if the socket is ever reachable by someone else is bigger
   than the norm this matched.

   - **Owner-only socket.** `ipc/listener.rs` sets the socket `0600`. Not
     theoretical: with `umask 000` the pre-change binary published
     `srwxrwxrwx` and another local user could both read state *and* inject
     keystrokes (`flexwm msg type` as `nobody` returned `Ok`); after, the mode
     is `0600` whatever the umask and that same call gets `EACCES`. Under the
     ordinary `umask 022` the socket came out `srwxr-xr-x`, which already
     refused other users (connecting needs write permission), so the exposure
     was umask-dependent, not unconditional — the Backlog entry's "non-default
     umask" half was the real one.
   - **Same-user peer check.** `accept()` compares the peer's uid against the
     compositor's own before the connection costs an fd, a buffer or a place
     in the event loop. This is a second, independent layer, proven
     independent on hardware: root bypasses the file mode entirely, connects,
     and is refused by the uid check (`Connection reset by peer`, with a
     `warn!` naming uid 0). An exact match, so root is refused too — it can
     reach the process by other means anyway.
     - **Effective, not real, uid.** Linux fills `SO_PEERCRED` from the
       peer's *euid* (`cred_to_ucred` uses `cred->euid`), so the comparison
       is against `geteuid()`; using `getuid()` would compare two different
       things in the one case they differ.
     - Two claims in the plan for this item turned out wrong in the code and
       were corrected rather than worked around: `UnixStream::peer_cred` is
       **not** stable (still `peer_credentials_unix_socket`, rust#42839 — the
       build failed on it), and rustix's `socket_peercred`, though rustix is
       already a dependency, reads the kernel's `struct ucred` straight into a
       `UCred` whose `pid` is a `NonZeroI32` — and the kernel writes pid 0
       for a peer in a PID namespace this process can't see it in
       (`pid_vnr`), a niche-invalid value that declining to read the field
       does not avoid. So the sockopt is asked for by hand through `libc`
       (already in the tree; a direct dependency, not a new crate), reading
       only the uid, with the struct pre-filled with the kernel's own `-1`
       sentinel so an unwritten one can never read back as a valid uid.
       `geteuid` has no such hazard and still goes through rustix's safe
       wrapper.
   - **Bounded request line.** `Connection::step` read through
     `BufRead::read_line` into an unbounded buffer, so one connection
     streaming bytes with no `\n` grew it without limit. Replaced with
     `ipc/line.rs`'s explicit `fill_buf`/`consume` loop and a 1 MiB cap
     (generous: the largest real request is `Request::Type`'s text, and a
     500 KB one still works). Measured, release builds, 200 MiB of
     newline-less garbage from one connection: before, compositor RSS
     10,540 kB → 215,344 kB (VmPeak 282,620 kB) and still waiting; after,
     the client's write dies with `EPIPE` at 1,114,112 bytes, gets an
     explicit error reply, and RSS stays at 10,736 kB (VmPeak 21,512 kB).
     `flexwm-ipc`'s shared `read_message` is deliberately **not** capped: the
     same codec reads `Response::Screenshot`, legitimately several MB of
     base64 PNG, so a global cap there would break real screenshots.
   - **Per-connection screenshot rate limiting.** A capture is a full render
     plus framebuffer read-back plus PNG encode on the one event-loop thread.
     A connection handed one less than one `FRAME_INTERVAL` (16ms, reused
     from `headless.rs`, not a new magic number) ago now gets a
     `Response::error` instead of another capture. Never a sleep — that would
     block every other client to slow one down; a refusal is answered in
     microseconds.
     **The first implementation stamped the clock when the request arrived
     and was a complete no-op, caught by bug-bashing it on hardware rather
     than by any test**: a capture takes longer than a frame (~170ms for
     800x600 in a debug build, ~12ms for 1600x1000 in release), so by the
     time the next request arrived the window had always already expired —
     50 back-to-back requests, 50 served, 0 refused. Stamping when the
     capture *finishes* is what makes the window mean anything, and is what
     the ticket actually wanted ("leave the event loop room after a
     capture"). Release, 1600x1000, 200 back-to-back requests on one
     connection: before 200 served in 2.45s costing 242 compositor jiffies;
     after 2 served / 198 refused in 48ms costing 2 jiffies. Per connection,
     so bypassable by reconnecting per capture — capping concurrent
     connections is the audit's separate finding and stayed out of scope.
   - **Atomic publish instead of unlink-then-bind.** `listener::bind` creates
     a `0700` directory beside the socket path with `mkdir`, opens it
     `O_DIRECTORY | O_NOFOLLOW`, binds the socket inside it as
     `/proc/self/fd/<n>/s`, chmods it `0600` there, and `rename`s it onto the
     published path, removing the directory either way. The Backlog called the
     old order a symlink race; tracing it showed that is not the exposure —
     `bind(2)` does not follow a symlink at the final component, confirmed
     empirically (a dangling symlink at the path makes bind fail `EADDRINUSE`,
     errno 98), and `remove_file` unlinks a link rather than its target. What
     it really had was a window where the socket existed at its published name
     with whatever the umask allowed before the `chmod` could run, plus a
     window where the name was missing and another process could claim it.

     This took three attempts, the first two of which `flexwm-reviewer` and a
     self-review caught as *worse* than what they replaced — worth recording,
     because each one failed for the same reason: a path that another user can
     write to cannot be re-resolved by name once it has been checked.
     (a) Binding at `<path>.<pid>.tmp` and chmodding that path:
     `set_permissions` follows symlinks, so they could swap the staged socket
     for a link between the bind and the chmod and have the compositor chmod a
     file of their choosing. (b) Staging inside `<path>.<pid>.tmp/` but
     *pre-cleaning* that name first: `remove_file` on `<staging>/socket`
     resolves `<staging>` as a non-final component, so a symlink planted there
     — cheaply, per-pid, in advance — deleted a file of their choosing through
     it, and then `mkdir` failed `EEXIST` against the surviving link on every
     subsequent start. (b) also cost 17 path bytes, which broke binding for
     any path over ~90 bytes, at a threshold that moved with the pid's digit
     count. The third shape fixes both classes at once: `mkdir` *is* the claim
     (it neither follows a symlink nor replaces a name, so nothing is ever
     removed to make room), an unpredictable `.flexwm-<12 random>` name means
     there is nothing to pre-plant at, everything after the claim goes through
     the pinned fd rather than the name, and `/proc/self/fd/<n>/s` is both
     unswappable and a fixed ~17 bytes regardless of the published path — so a
     107-byte socket path, the longest `sun_path` allows, binds. All three
     primitives were verified on the dev VM before the code was written, not
     assumed: `O_DIRECTORY | O_NOFOLLOW` against a symlink-to-a-directory
     fails `ENOTDIR`, `File::set_permissions` fchmods a read-only directory fd,
     and `rename` out of `/proc/self/fd/<n>/` onto a 107-byte path works and
     stays connectable.

     The umask is the other way to get a `0600` socket with no chmod at all,
     and was the second attempt's successor before being abandoned mid-flight:
     it is process-global, and the test suite proved that is not academic —
     `tempfile::tempdir()` in a *concurrent* test, created inside the umask
     window, came out mode `0600`, which for a directory means untraversable,
     and unrelated tests failed with `EACCES` in one run out of three. A
     hazard that live in-process is not something to ship behind a comment.

   **Owner-only socket, restated for the final shape:** the mode is set on
   the socket while it is still inside the staging directory, so it is `0600`
   before it is reachable under its published name at all — there is no window
   at the published path, whatever the umask.

   `ipc.rs` split into `ipc/line.rs`, `ipc/listener.rs` and `ipc/tests.rs`
   before it sprawled. 128 tests (26 new, against the 102 on the merge
   base), clippy/fmt clean, `cargo test`
   green workspace-wide, `scripts/smoke-test.sh` green under both `--headless`
   and `--nested`, and the whole bug-bash re-run against real `--tty` hardware
   (it is IPC-layer code, so it is backend-agnostic by construction — but
   confirmed, not assumed). Per-request cost of the bounded read, measured
   because it is on the per-request path: release, 50,000 `version`
   round-trips, 6 interleaved
   reps per side — before mean 122.42us/66.5 jiffies, after mean
   122.03us/66.0 jiffies, fully overlapping. (Debug builds showed a consistent
   ~5% jiffies gap, which is a debug-build artifact: std's `read_line` uses
   `memchr` where this loop uses a plain byte scan, and only the unoptimized
   build can tell.)

   **`flexwm-reviewer`'s pass found two blocking regressions, both in
   `listener::bind`, both fixed as described above** — and both in the part of
   the change that was *new* rather than in the four audit fixes themselves,
   which it re-derived and confirmed (`geteuid` over `getuid`, `libc` over std
   or rustix, completion-stamped throttling, the 1 MiB cap and the peer check
   under adversarial testing, no measurable benchmark regression). It also
   found three smaller things, all addressed: the throttle's justification
   overclaimed in two places (`FRAME_INTERVAL`'s doc and the client-facing
   error both said the screen "cannot have changed", which `render()`'s
   on-demand scheduling does not guarantee — reworded to the claim that is
   true, that it bounds what one connection can cost the event loop), this
   entry and the PR description still described the superseded design, and the
   deferred blocking-I/O finding below was under-rated at MEDIUM.

   **Three pre-existing defects found while bug-bashing this, deliberately not
   fixed here** — all verified identical on the pre-change binary, so none is a
   regression, and all are one Backlog entry below (closed as item 10): a
   half-written request line
   blocks the entire event loop for as long as the client holds it (every other
   client included), a second request pipelined into the same write is never
   answered, and a client that never reads its replies deadlocks the loop from
   the write side. One root cause (the connection does blocking I/O and reads
   one line per readiness event), one fix, and that fix restructures the
   connection loop — its own item, not a rider on this one.

10. ~~The IPC connection loop did blocking I/O, one line per readiness
    event~~ — DONE, PR #15. The Backlog's one HIGH entry (see below, left
    as-written), picked up ahead of item 6 for the reason it says: a client
    needs no malice and one byte of effort to freeze the whole compositor.

    **What shipped**, by symptom:

    - **(a) a half-written request line.** The accepted socket is now
      non-blocking, and `ipc/line.rs`'s reader owns its line buffer rather
      than borrowing one from the caller — because the two are coupled in a
      way that is easy to get wrong: a line split across reads must *keep*
      its prefix, so the buffer can be cleared exactly once per line, after
      the caller has consumed it, never once per read. `WouldBlock` became
      `LineRead::Incomplete`, a third outcome distinct from both an error and
      a real end of stream, so a client that goes quiet mid-line is never
      confused with one that went away mid-line (the latter still gets that
      last line answered, as before). The 1 MiB cap now applies across
      however many reads a line arrives in — the test for that is the one
      that would have caught preserving the prefix as a way around the cap.
    - **(b) a pipelined second request.** `Connection::step` answers every
      line already in its read buffer before handing the thread back. That half
      is forced: lines already pulled into userspace are never reported again by
      a level-triggered source, so leaving one there is the bug itself. What
      bounds the other side -- how long one connection may hold the only thread
      the compositor has -- is a budget of *reads off the socket* per wakeup
      (`READS_PER_WAKEUP`, one), which is the only thing that can. See the
      blocking correction below for why, and for the 32ms stall two earlier
      versions of this shipped with.
    - **(c) a client that never reads its replies.** `ipc/outbound.rs` queues
      whatever of a reply the socket would not take and finishes it when the
      event loop reports the socket writable. A connection with anything
      queued is registered for writability *only*, which is the flow control:
      it cannot queue more work while it is behind, and a level-triggered
      READ registration on bytes it has decided not to read would spin the
      loop at full speed.

    **Design decisions worth the reviewer's attention**, since the Backlog
    entry's fix direction left them open:

    - **The outbound cap is a high-water mark, not a limit, and nothing is
      ever refused, truncated or closed for being slow.** The read side's
      1 MiB number is not reusable here: a request's size is client-chosen
      and nothing legitimate needs a megabyte, but a *response* is sized by
      the compositor — a `Response::Screenshot` is legitimately several MB of
      base64 PNG, and a client reading it slowly on a loaded machine is slow,
      not hostile. So `HIGH_WATER_BYTES` (1 MiB, its own constant with its own
      rationale) only bounds what a *single wakeup* may add to the queue,
      which is the one thing the write-only interest cannot bound: the
      requests that wakeup is working through have already been read. Net
      bound per connection: 1 MiB plus the one response that crossed the mark
      — and that response was already in memory to be written, so this costs
      no more than the `write_all` it replaces, which held the same bytes for
      as long as it blocked. No timeout, and no "close a slow client", for
      the same reason.
    - **The consequence, stated plainly:** a client that pipelines more
      unread requests than its own `SO_SNDBUF` holds, while never reading the
      answers, stalls — itself, alone. That is the same threshold at which
      the blocking code deadlocked, except it used to take the whole
      compositor with it. Demonstrated on hardware (below) rather than
      reasoned about.
    - **`Connection::closing`** (set by end-of-stream and by an over-long
      line) means "no further request will be read", *not* "close now": the
      connection stays in the loop until its queue has drained. Without that,
      the end of a client's requests would also be where its replies got
      discarded — a client that writes a request, shuts its write half down
      and waits for the answer is a perfectly ordinary client.
    - **`wait-idle`'s hand-off takes the connection's outbound queue with
      it.** That request removes its own source from the event loop and
      answers later from the render loop through a cloned fd, so anything
      still queued on the connection would have nothing left to write it —
      and the idle answer, written through the same socket, would land in the
      middle of a half-written response. The `PendingIdle` carries the queue,
      and its own write is non-blocking too (`try_clone` shares file status
      flags, verified), retried per frame tick, given up on only after the
      client's own `timeout_ms` has passed with *no write progress at all*
      rather than after a fixed deadline — a client draining a multi-megabyte
      reply slowly is making progress and is never given up on. The cost of
      that retry, named because this project benchmarks idle CPU: while such a
      reply cannot drain, `frame_tick` keeps rescheduling at 16ms (`render()`
      early-returns on `!needs_render`; the retry is one `EAGAIN` write per
      tick) for up to that `timeout_ms`. Not a regression — a `wait-idle` with
      a huge timeout over a never-settling screen already held the timer
      exactly the same way — and bounded by what the client itself asked for.
    - **`ConnectionSource`**, a small `EventSource` wrapping `Generic`, exists
      because `Generic`'s callback cannot reach the `Generic`'s own `interest`
      field, and switching interest from inside the connection's own callback
      is the whole point. Both halves are needed and in this order: set the
      field (what `reregister` reads) and return `PostAction::Reregister`
      (what gets `reregister` called). Checked against calloop 0.14.4's own
      source — `loop_logic.rs`'s post-callback handling, `generic.rs`'s
      `reregister`, and `token.rs` for the fact that the re-derived token is
      identical, so no event can be lost to the switch.

    **One correction to the diagnosis below:** its "with a cap on that buffer,
    since it is the same unbounded-growth shape item 9 just closed" reads as a
    hard cap, which would have been wrong for screenshots -- see above.

    **And one correction to two earlier versions of this entry, which
    `flexwm-reviewer` caught as a blocking finding.** They claimed "one read
    buffer is the bound on how long one connection can hold the thread", and
    justified deleting an explicit per-request counter on the grounds that the
    read buffer had been bounding a wakeup all along. **Both were false, and the
    second is why the first went unnoticed.** `Lines::next` has its own loop
    around `fill_buf`, because a line that ends mid-chunk can only be finished
    by reading again -- so a serve loop that continues "while something is still
    buffered" runs until a chunk boundary happens to land on a line boundary,
    i.e. for `lcm(chunk, line length)` bytes, which for a line length coprime
    with the chunk is the whole flood. Neither lines served, nor bytes served,
    nor the buffer's size bounds that; only counting reads does, which is what
    `READS_PER_WAKEUP` is.

    Measured, because the first write-up reasoned where it should have
    measured. One ordinary client doing strict request/response round-trips
    while another floods with 160 KB batches of 101-byte requests; release
    build, 6s samples:

    | | p50 | p90 | p99 | max | round-trips |
    |---|---|---|---|---|---|
    | quiet (either way) | ~126us | ~155us | ~242us | 1.8ms | 23,608 |
    | flooded, no read budget | **31,975us** | 34,768us | 35,988us | 38.6ms | 195 |
    | flooded, one read per wakeup | **358us** | 390us | 553us | 5.6ms | 17,462 |

    Two frames of frozen input, dispatch and rendering per wakeup, repeating
    for as long as the flood lasts -- a materially weaker version of "only the
    offending client is affected", which is the whole point of this item. The
    bound costs the flooding client about 9% of its own throughput (423 MB vs
    386 MB of requests pushed in 8s) and gives the innocent one 90x the
    round-trips at 65x better p99. Reproduced on real `--tty` hardware to the
    same figures (p50 353us, p99 553us), so it owes nothing to the backend.

    The test that covers this had to be written at that scale *and* with that
    line length: at 600 19-byte `version` requests it passes either way,
    because this kernel hands out 2641-byte socket chunks and 19 divides 2641
    exactly, so even the unbounded loop yields at the first boundary. That is
    also why two rounds of review missed it -- every earlier test used 19-byte
    requests.

    A third claim was **confounded rather than false**: the "+2 jiffies per
    50,000 round-trips" attributed to the read-until-`Incomplete` shape was
    measured with a biased design (the same binary always first in a rep), and
    a balanced one shows the run *position* is worth about that much on its own
    (whichever binary runs second in a rep averages +1.15us and +1.36 jiffies).
    The extra `EAGAIN` syscall per request was real and provable by inspection,
    and removing it was right, but its cost was never measured apart from the
    artifact.

    **And one more finding, introduced by that very fix.** Hoisting
    `flush_clients()` out of the per-line loop left two exits that skipped it:
    `serve` returning `Step::Close` -- the `wait-idle` hand-off, or a reply that
    could not be written -- and a failed read, both of which returned straight
    out of `step`. `wait-idle` makes that a correctness bug rather than a
    latency one, on exactly the pipeline an agent writes: `type` and `wait-idle`
    in one write. The keystroke's wayland messages are queued by the time
    `serve` returns, the early return skipped the flush, and nothing else
    flushes them -- the frame timer's `render()` returns immediately unless
    something marked the screen dirty, and injected input does not. So the
    client could not have redrawn for a key it never received, and
    `idle_outcome` answered `idle` over a screen that had not changed yet: the
    stale-idle race its own doc comment is about. Every exit now breaks rather
    than returns, `served` is set *before* `serve` runs (a lone `wait-idle`
    serves one request and then closes), and the flush is the single point they
    all pass through.

    Shown on hardware rather than argued, against a release build of the same
    tree with that one exit returning early again: spawn `foot`, let it settle,
    capture, then one 106-byte `write()` of `type 'echo
    pipelined-flush-probe'` + `wait_idle quiet_ms=300`, then capture again the
    moment the idle answer comes back. `magick compare -metric AE` between the
    two captures: **586 pixels changed with the fix, 0 without it** -- and 586
    either way once the screen had settled, so the keystroke was never lost,
    only late. `waited_ms` was ~305 in both runs: the clock cannot tell the two
    apart, which is why this is measured on the screen.

    **Deliberately out of scope, named because they are adjacent:**
    `wait-idle` is still terminal for its connection, so a request pipelined
    *after* one is silently never read (pre-existing, unchanged). A refused
    over-long request is followed by `ECONNRESET` rather than a clean EOF,
    because the compositor stops reading the rest of the line and Linux resets
    a unix socket closed with unread data — the refusal itself still arrives
    first (the kernel reports the error only once the receive queue is empty),
    and a client mid-`write_all` past 1 MiB sees the write fail rather than
    the refusal. Both pre-existing and both unchanged here. Capping concurrent
    connections is still its own Backlog entry, now with a new lifecycle case
    recorded next to it (found by `flexwm-reviewer` reviewing this item): a
    half-closed, never-reading client pins a connection slot and two fds for
    good -- strictly better than the pre-item-10 behavior, where that same
    case froze the whole compositor, but still a live leak. See that Backlog
    entry for the mechanism.

    **Tests: 47 new (175 total, against 128 on the merge base).** Split per
    module: `line/tests.rs` (partial lines, the cap across reads, the three
    end-of-stream cases, the read budget), `outbound/tests.rs` (partial writes,
    reply ordering, what the mark measures, a drained queue freeing its
    buffer), `tests.rs` (the `wait-idle` answer's retry-and-give-up logic,
    which nothing outside the compositor can observe: it is driven directly,
    with a deliberately tiny `SO_SNDBUF` so the answer cannot go out), and
    `connection/tests.rs`, which drives a real `State` through a real
    `EventLoop` for all three symptoms plus the per-wakeup bound, the
    `wait-idle` hand-off, the screenshot limiter across a pipelined write, the
    graceful close, the interest registration and the per-wakeup wayland flush.
    That last one needs a wayland client to observe, and uses the cheapest thing
    that can be one: a raw socket handed to `insert_client`, one `get_registry`
    written by hand, and then a new global created to queue a
    `wl_registry.global` for it -- which nothing flushes until something
    chooses to. A single `wait-idle`, with nothing pipelined ahead of it, is
    then the only thing that can have flushed it. (A keystroke would be the
    more literal fixture, but it needs keyboard focus, i.e. a mapped toplevel
    and a real toolkit client; what is under test is the flush, not what filled
    the buffer, and the hardware run above covers the literal case.) Those hand
    `accept()` one end
    of a socket pair rather than going through the listener -- the peer is then
    this process (so the uid check passes) and, the reason that matters,
    `SO_SNDBUF` can be shrunk on the compositor's end before it is handed over,
    which is the only way a test makes a reply not fit in one write without
    pushing megabytes through a debug build. The client and compositor share
    one thread, so a test that *would* block hangs rather than fails -- which
    is the correct outcome, and is what the negative controls below exercise.

    **Thirteen negative controls, each a one-line mutation, run on the dev VM;
    every one failed a test that passes against the real code**, which is what
    makes the suite non-vacuous rather than merely green. Leaving the accepted
    socket blocking hangs the partial-line test, and separately the
    never-reads test, past 60s instead of failing -- exactly symptoms (a) and
    (c). Clearing the line buffer on `WouldBlock` fails five tests. Serving one
    line per wakeup fails the pipelining and screenshot-limiter tests with
    "only 1 of 2 replies arrived" -- exactly symptom (b). Handing `wait-idle` an
    empty queue instead of the connection's fails the framing test at 7 of 301
    replies. Closing on end-of-stream without draining loses 305 of 400 queued
    replies. Removing the read budget answers 278 of 1,622 pipelined requests
    in one wakeup, against a bound of 82. Keeping a big line buffer instead of
    freeing it leaves 131,072 bytes alive after the line that needed them. And
    each of three independent ways of breaking the `wait-idle` write-progress
    logic -- give up on the first stalled tick, never record progress, never
    start the clock when the answer is queued -- fails between one and three of
    the four tests written for it, including both mutations the review found
    passing everything. And the two ways of losing the per-wakeup flush --
    returning from the serve loop instead of breaking out of it, and marking a
    request served only after `serve` returns -- each fail the flush test and,
    run against the whole suite, only that test (174 passed, 1 failed, both
    times), which is what makes both halves of that fix load-bearing rather than
    one half plus a tidy-up. Raw output in PR #15.

    One change is deliberately **not** covered by a test, and says so in its
    comment: resetting `sent` alongside emptying the buffer in
    `Outbound::send` guards an invariant that holds today, so nothing can
    distinguish it behaviourally -- the `debug_assert` in `pending()` is what
    would catch a future violation.

    **Hardware-verified on real `--tty`** (dev VM, virtio-gpu KMS at
    1600x1000, release binary, **re-run in full at `41b7a50`** -- both review
    rounds changed the serve loop, so every earlier run's cache key is stale and
    every figure in this entry is from the last one): `/proc/<pid>/wchan` read
    `do_epoll_wait` throughout -- never `unix_stream_read_generic`, the
    observable this item was diagnosed by. While one client held
    `{"type":"vers` for 20 seconds: three round-trips served (361/189/196us),
    `flexwm msg windows`, a full 33,476-byte screenshot and a `wait-idle`
    (`waited_ms: 203`) all answered, and the compositor burned 0 jiffies over
    the whole window. Four clients holding partial lines simultaneously were
    each answered their own reply. A client that wrote 14,136 bytes of requests
    and read none (back-pressure engaged -- its own writes stopped being
    accepted) delayed nobody, and when it finally read: 37,200 bytes, 744 reply
    lines, **0 damaged**, exactly the 744 expected, so an interleaved write
    never corrupted the framing. The flood-versus-round-trip figures above hold
    here too (re-measured at this SHA: quiet p50 125us/p99 244us over 23,180
    round-trips, flooded p50 353us/p90 384us/p99 553us over 17,943 while the
    flooder pushed 390.6 MB in 8s). Idle CPU 0 jiffies over 10s both before and
    after everything, so neither the interest switching nor the read budget
    introduced a spin; one WARN in the whole log, smithay's own
    `Failed to destroy old mode property blob` at modeset. The 1 MiB request cap
    re-verified against the same release binary over a real socket: the flood
    dies at 1,114,112 bytes, the refusal arrives intact first, RSS stays at
    10,668 kB (VmPeak 21,520 kB -- item 9's figures), the compositor survives
    and a fresh connection still works (195us).

    **Benchmarked** (item 9's method: release, 50,000 `version` round-trips
    over one connection, compositor jiffies plus us/round-trip) **with the run
    order balanced**, because position within a rep turned out to be worth more
    than the change being measured. Re-run at `41b7a50`: 20 reps per side, 10
    with the pre-change binary first and 10 with it second. Pooled medians:
    **126.03us/71 jiffies after versus 127.57us/71 before**. Three of the 40
    runs came in under 110us (79.7, 81.5, 88.3) -- one on the pre-change side,
    two on the other, which is what makes them VM scheduling noise rather than
    a property of either binary, and why this is reported as medians. No
    measurable difference on the uncontended round-trip path, which is the path
    the fairness budget adds a comparison to; the same answer the first round's
    28-reps-per-side run gave, with the sign flipped.

    `scripts/smoke-test.sh` green under `--headless` and `--nested` (under
    `cage`), 175/175 tests, clippy and fmt clean. The whole compositor is
    `#[cfg(target_os = "linux")]`, so none of this can be verified from the Mac
    host -- every figure here is from the dev VM guest.

11. ~~A real keystroke could sit unflushed to the client for seconds on a quiet
    screen~~ — DONE, PR #17. The Backlog's MEDIUM entry (left as-written below),
    found by `flexwm-reviewer` while reviewing item 10 and the same root cause
    item 10 fixed for IPC-injected input only. Ahead of item 6 for the usual
    reason: small, self-contained, already diagnosed.

    **The fix is one line in `compositor/mod.rs`**: `event_loop.run`'s
    post-dispatch callback was `|_| {}` and is now `post_dispatch`, which calls
    `display_handle.flush_clients()`. That makes "anything queued during a
    wakeup goes out at the end of that wakeup" structural rather than a
    discipline to remember at each call site -- which is what had been missed:
    `tty/mod.rs`'s libinput callback and `nested_dispatch.rs`'s host-forwarded
    equivalent both hand a real key to `input::key`, which queues the client's
    `wl_keyboard.key` and nothing else. A keystroke does not mark the screen
    dirty, so `render()` early-returns on `!needs_render` before its own flush,
    and the frame timer has already dropped itself when idle.

    Three claims behind the suggested fix were checked rather than taken as
    given, and one needs restating:

    - **calloop 0.14.4's `EventLoop::run`** is literally `while !stop {
      dispatch(timeout)?; cb(data); }` (`loop_logic.rs:657`), `cb: FnMut(&mut
      Data)`. So the callback runs once per wakeup -- and with flexwm's
      `timeout` of `None`, `dispatch` blocks until a source is ready, so an idle
      compositor gets no wakeups and therefore no flushes. That is the whole
      idle-CPU argument.
    - **"Matching anvil's pattern" is true in substance, not in form.** Anvil
      (`anvil/src/udev.rs:520-545`, `winit.rs`, `x11.rs`) does not use `run` at
      all: it hand-rolls `while running { dispatch(Some(1ms|16ms)); space
      .refresh(); popups.cleanup(); flush_clients().unwrap() }`. Same
      flush-after-every-dispatch-cycle shape at the hook `run` provides
      instead; flexwm's `space.refresh()`/`popups.cleanup()` stay in `render()`,
      and unlike anvil's timeout-driven loop, `run(None, ..)` does not poll at
      idle.
    - **The cost of a flush with nothing queued**, re-derived from
      wayland-backend 0.3.17 rather than from the review's characterization of
      it: `flush_clients` → handle `flush(None)` → `for client in clients_mut()
      { let _ = client.flush() }` (`rs/server_impl/handle.rs:63-74`) →
      `BufferedSocket::flush`, whose write loop is `while written_bytes <
      bytes.len()` (`rs/socket.rs:155`) -- zero syscalls on an empty buffer,
      then `offset(0)`/`move_to_front()`/`drain(..0)`. So it is one mutex plus
      O(clients) of bookkeeping per wakeup.

    **`ipc/connection.rs` is deliberately untouched**, including its
    hand-maintained "every exit reaches the one flush" invariant, which this
    change does make redundant for user-visible staleness. It is well-tested,
    cost three review rounds, and each of the older flush sites still flushes
    *earlier within its own wakeup* than this one does (the display source
    before the loop moves to another source, `render()` after a frame's frame
    callbacks, `step()` before a reply's own round trip can race it) -- so the
    overlap buys lower latency for free. What is now stale is only the
    *justification* written at two of them ("nothing else flushes until ..."),
    recorded here rather than edited in: something else does now, just later.
    `post_dispatch`'s own doc comment names all three and says why each stays.

    **Tests: 2 new (177 total, against 175 on the merge base)**, in a new
    `compositor/tests.rs`. Both drive a real `State` with a real wayland client
    through a real `EventLoop::run` -- the only thing that can observe *when* a
    queued message leaves, and the reason `run` is called with the same
    `post_dispatch` function production uses rather than a copy of it. The
    client is a raw socket handed to `insert_client` with one hand-written
    `get_registry`, and the queued message is a `wl_registry.global` from a
    newly created output: item 10's `connection/tests.rs` established that
    fixture for the same reason (a keystroke needs keyboard focus, a mapped
    toplevel and a real toolkit; what is under test is the flush, not what
    filled the buffer). `Timer::immediate` + `loop_signal.stop()` is what lets a
    `run(None, ..)` return after exactly one cycle -- the stop flag is only
    checked *after* the callback, so the flush under test does happen. The
    second test asserts a dispatch cycle *without* the callback leaves the
    message queued, which is what keeps the first non-vacuous, and the
    **negative control** confirms it directly: passing `|_| {}` (the production
    code as it shipped) fails the first test with "the message queued before the
    dispatch cycle never reached the client" and leaves the second passing.

    **Hardware-verified on the dev VM's real `virtio-gpu` KMS device at
    1600x1000**, release builds of `main` at `9cf8b9e` and this branch at
    `0de11d2` built from `git archive` into separate trees and separate
    `CARGO_TARGET_DIR`s (the staleness hazard in `HANDOFF.md`), with a real
    `/dev/uinput` keyboard injecting one `KEY_X` and holding the virtual device
    alive afterwards so the key is the *only* libinput event in the measured
    window. Quiet-screen control first: two captures 1.5s apart with no input
    at all, `magick compare -metric AE` = 0. Then, after the key:

    | | capture at ~40ms | capture at ~1.5s |
    |---|---|---|
    | `--tty` before | AE 0 (not there yet) | AE 217.238 |
    | `--tty` after | **AE 217.238** | AE 217.238 (nothing changed since) |
    | `--nested` before | AE 0 | AE 217.238 |
    | `--nested` after | **AE 217.238** | AE 217.238 |

    The same 217 pixels either way -- the one `x` foot drew -- so the keystroke
    was never lost, only late, and "late" was until the *next* screenshot's own
    end-of-wakeup flush delivered it. `--nested` ran under `cage` on the same
    real DRM device, with the host forwarding the injected key, which is the
    other input path the Backlog entry names; it is the same event loop
    (`nested.rs` inserts `WaylandSource` into the same `loop_handle`, and
    `mod.rs`'s `run` is the compositor's only *production* `run`/`dispatch`
    call site -- the test modules have their own, deliberately), so no separate
    render loop can bypass this.

    **Benchmarked**, because this runs on every wakeup. Raw numbers, all
    interleaved with the run order balanced (item 10's correction):

    - **Idle CPU, the concern that matters most: 0 jiffies over 60s, both
      binaries, twice each**, with a mapped `foot` and a settled screen. `run`
      blocks in `dispatch(None)` at idle, so there is nothing to flush and
      nothing to wake for.
    - **A real 1000 Hz-class pointer-motion flood — corrected by
      `flexwm-reviewer` after the original benchmark below turned out to
      measure nothing.** The output is 1600x1000 and the original run parked
      the pointer at the *output* centre, `800,500` -- 6px outside the
      single test window's actual bounds (`x ∈ [12,794]`). `strace -c -f` on
      the client during a 3000-event flood at that position recorded **zero
      syscalls in 12s**: the compositor queued it nothing, so `post_dispatch`'s
      flush was an empty-client-list walk in *both* builds, and the "two
      independent sets disagree in sign" result below was an artifact of
      whichever binary happened to run first in a group being slower --
      not a real before/after difference. Re-run with the pointer verifiably
      inside the window (`400,500`), untraced, balanced run order, 10,000
      events, measuring on-CPU time directly (`/proc/<pid>/task/*/schedstat`,
      nanosecond resolution, not 10ms jiffies): compositor on-CPU **before
      mean 884.0ms (sd 59.6, n=7), after mean 1023.3ms (sd 63.9, n=7) -- a
      +139.3ms/10k-events (+13.9us/event, +15.8%) increase, Welch t=4.22,
      p≈0.001**. Client on-CPU rose ~21.7% (p≈0.02) over the same runs.
      **This is real, not noise, and it's real for exactly the reason the
      original bullet named**: motion used to batch into the frame tick's
      flush; now every wakeup's events go out at the end of it. Client
      wakeups confirm the mechanism directly (`strace -c` on `foot`, 3000
      events): `epoll_pwait` 465 → 2696 (5.8x), `recvmsg` 930 → 5424.
      **In absolute terms this is small**: at the ~435 events/s this flood
      actually delivered, that's +6.0ms compositor CPU per second of
      sustained flooding, ≈0.6% of one core -- roughly +1.4% extrapolated to
      a real 1000Hz mouse. Verdict: an accepted, real cost in exchange for
      lower per-event latency, matching Smithay's own `anvil` reference
      compositor's pattern (verified directly against the pinned checkout:
      `anvil/src/udev.rs:537-546` and `winit.rs:449-456` both hand-roll
      `dispatch(...)` → `flush_clients()` once per loop iteration, same
      shape as this fix). Caveats worth keeping in mind before citing these
      numbers elsewhere: this VM's syscalls are more expensive than real
      hardware's, so 13.9us/event is likely a ceiling, not what real
      hardware would show; the 5.8x client-wakeup multiplier is
      hardware-independent and won't shrink; and "after" runs finished ~5%
      faster in wall time across the whole benchmark (an unexplained VM
      scheduling artifact) in a direction that would bias the measured
      *increase* downward, so +15.8% is if anything an underestimate, not
      an overestimate. (The `libwayland`/`wl_display_run` comparison in the
      original bullet was not independently verified -- treat it as
      unconfirmed, not as corroborating evidence.)
    - **What that costs the client, since the compositor's own jiffies cannot
      show it**: see the wakeup-multiplier numbers just above (5.8x more
      `epoll_pwait`/`recvmsg` calls under flood) -- the original "0 jiffies
      over 20,000 motion events in both builds" claim here shared the same
      out-of-window fixture bug and measured the same empty-client-list
      walk, not real client-side cost.
    - **The same flood at maximum rate** (50,000 events in ~300ms, ~160k/s):
      before 7/10 jiffies, after 6/11, from the same original benchmark run
      as the corrected bullet above -- likely shares its out-of-window
      fixture bug and has **not** been independently re-verified. Treat this
      specific number as unconfirmed rather than as evidence the cost
      vanishes at high rates; the corrected, verified numbers are the ones
      above.
    - **IPC round-trips** (50,000 `version` requests, one wakeup each), with
      three `foot` clients connected so the flush actually walks a client list:
      before mean 126.22us/71.3 jiffies, after 125.01us/69.8 -- after slightly
      *faster*, i.e. noise. With no wayland clients at all: 125.12us/69.8
      before, 124.75us/68.8 after.

    **Bug-bashed on the same hardware**, fixed binary, all six scenarios alive
    with no panic and no log line beyond smithay's pre-existing
    `Failed to destroy old mode property blob` WARN at modeset: a real key with
    **zero windows** (no client to flush to at all; still answers IPC and
    renders afterwards); the focused client **`kill -9`'d 32ms after the key**,
    i.e. around the flush, after which a fresh client is served normally;
    **2,000 real press+release pairs in 21ms** (3 jiffies, 20,748 pixels
    changed); a key arriving while the session is **VT-paused** (no `EPERM`,
    still answers IPC, and after `chvt 1` it renders again and a fresh key
    changes the screen at the fast timing); **`type` + `wait-idle` pipelined in
    one 100-byte write** (both replies, 504 pixels changed by the time the idle
    answer came back -- item 10's invariant unregressed); and **three clients
    with one `kill -9`'d** (2 windows left, a real key still reaches the focused
    one).

    Traced against the frame-tick machinery it sits next to, since they are
    easy to confuse: `post_dispatch` only flushes. It never touches
    `needs_render` or `timer_armed`, so it cannot cause a render, keep the frame
    timer alive, or interact with `ensure_ticking`/`frame_tick`'s
    drop-when-idle logic -- which the 0-jiffy idle measurement confirms
    empirically. `flush_clients` is also the wayland-*server* side only: the
    `--nested` host connection and `wait-idle`'s own cloned-fd writes are
    untouched by it.

    No README change: nothing user-facing moved -- no flag, config key, request
    or documented behavior -- and the Status section describes capabilities, not
    latency bugs it no longer has.

12. ~~Four missing bounds: a panicking startup path, an unclamped client
    `min_size`, an unbounded `gap`, and an unbounded `wl_shm` pool~~ — DONE,
    PR #18. Four Backlog entries (one MEDIUM, three LOW — all struck below)
    landed together because they are the same shape of fix in four small
    places: a bound, a clamp, or a clean error path instead of a panic. Ahead
    of item 6 for the same reason items 7-11 were.

    **(a) `State::listen` no longer panics when the wayland socket cannot be
    created.** `State::new` and `listen` return
    `Result<_, Box<dyn Error>>` (the two `insert_source().expect()`s inside
    `listen` became `?` with them, via `.map_err(|e| e.error)` — `InsertError`'s
    own payload is the source that could not be inserted, which is no use to an
    operator and would force the error type to carry a
    `ListeningSocketSource`), and a new pure `socket_error` maps every
    `BindError` variant to an operator-facing message. Five call sites traced
    and updated: `compositor::run` (propagates, so `main` prints
    `flexwm: <message>` and exits `FAILURE`) plus the four live-`State` test
    harnesses. `seat.add_keyboard`'s own `expect` in `State::new` was left
    alone deliberately — a different failure (no xkb keymap for the default
    layout), not this ticket.

    Before/after on the dev VM, **release** builds both ways (what makes the
    difference visible: `panic = "abort"`), `env -u XDG_RUNTIME_DIR flexwm
    --headless` — before: `panicked at state.rs:196: a free wayland socket:
    RuntimeDirNotSet`, `Aborted (core dumped)`, exit 134, and a real
    174.2K coredump recorded by `systemd-coredump`; after: `flexwm: no wayland
    socket: $XDG_RUNTIME_DIR is not set or invalid; set it to a writable
    directory (a login session normally provides one)`, exit 1, no coredump. A
    second scenario (`XDG_RUNTIME_DIR=/proc`, i.e. set but unwritable) prints
    the `PermissionDenied` message — traced to wayland-server's
    `bind_absolute`, where a lockfile it cannot create is mapped to that
    variant, and where `bind_auto` returns it immediately instead of trying the
    other 31 names.

    **(b) A client's `min_size` is clamped where `shell.rs` reads it**, to the
    usable area (`area.inset(gap)`) of the largest output the core knows —
    `learned_min`'s existing bound, widened from "the window's own output" to
    "any output" because nothing in `World`'s public surface says which output
    an unplaced toplevel will land on, and narrowing it further could wrongly
    shrink a hint a window could legitimately fill its own screen with. With
    the one output this compositor creates the two bounds coincide.

    **The backlog entry's diagnosis was one step off, found by disabling the
    clamp and watching the tests fail**: `ring_rects`'s `rect.w + 2 * width` is
    not the first unchecked add an `i32::MAX` minimum reaches.
    `flexwm_core`'s own `World::place_workspace` (`arrange.rs`, `x + width` in
    the on-screen test) overflows first, during `arrange`, before anything
    renders — a debug-build panic, a silently wrong `visible` flag in release.
    Both are closed by clamping at the read site.

    Two further facts the fix turned on: `xdg_toplevel.set_min_size` is
    unvalidated at the pinned rev (`handlers/surface/toplevel.rs` stores
    `(width, height)` verbatim, negatives included), and it is
    double-buffered — so it only becomes the `current()` value `info_of` reads
    on commit, and `info_of` itself only runs from `add_window` (on
    `get_toplevel`, before any commit) and `refresh_window`
    (`app_id_changed`/`title_changed`). The live test drives exactly that
    sequence, which is also what a real toolkit produces.

    **(c) `flexwm_core::Config`'s `gap` is bounded above as well as below.**
    `Config::MAX_GAP` = 10,000 px, past the long edge of an 8K display (7680),
    so no real output has usable area left at it; `Config::clamp_gap` is public
    because two places must agree on it — `validated()`, and `flexwm`'s config
    loader, which sizes the focus ring against half the gap *before* a `World`
    exists to validate one. This is not just a visual-proportionality fix:
    sizing the ring against the raw, unclamped gap means `focus_ring_width =
    i32::MAX` at `gap = i32::MAX` clamps to `gap.max(0) / 2 = 1073741823`
    (`Appearance::clamped`, unaware the gap itself is out of range), and
    `decorations::ring_rects`'s `rect.w + 2 * width` then overflows for any
    `rect.w >= 2` on every single render — a debug panic or a release
    wraparound, from a config file alone, found by `flexwm-reviewer` while
    checking this fix and covered by a dedicated test
    (`an_out_of_range_ring_width_is_also_clamped_against_the_capped_gap`).
    The doc comment deliberately does **not** claim the cap bounds
    `gap * (windows - 1)` in `column_heights`: that product is bounded by
    window count, not by this.

    Live on the dev VM, release binaries both ways, `gap = 2147483647` with
    one `foot` window on a 640x480 output, read back over IPC — before:
    `rect {x: 2147483647, y: 2147483647, width: 1073742145, height: 482}`
    (wrapped garbage; `2 * gap` in `Rect::inset` wraps, and that `x + width`
    then overflows too); after: `{x: 10000, y: 10000, width: 1, height: 1}` —
    degenerate, as a 10,000px gap on a 640px output should be, but coherent.
    `gap = 12` on the same binary still gives `{12, 12, 302, 456}`, so ordinary
    configs are untouched.

    **(d) `wl_shm` pools are capped at `MAX_SHM_POOL_BYTES` = 512 MiB**, at
    *both* requests that reach the same `mmap`: `wl_shm.create_pool`'s initial
    size and `wl_shm_pool.resize`'s target. Extends `dispatch.rs`'s existing
    hand-written blanket `Dispatch` impl (item 7's seam — the pinned rev has no
    `delegate_shm!`). 512 MiB is four full-screen 8K ARGB frames (126.6 MiB
    each) or sixteen 4K ones in a single pool; it is a per-pool bound, not a
    total, and the entry says so rather than overclaiming. `flexwm-reviewer`
    confirmed the total is still genuinely unbounded, live: 40 pools at
    exactly the cap (well within it individually) reserve ~20 GiB from one
    connection, more than the pre-fix 8-pool/16.1 GiB finding this item was
    written to close, just needing more requests to get there. Bounding the
    sum needs per-client accounting this module doesn't have — **not**, as an
    earlier version of this note said, "the separate connection-cap entry's
    territory" (that entry is about the IPC control socket's own connection
    count, unrelated to wayland client accounting). Tracked as its own
    Backlog entry instead.

    **Refused, not clamped**, deliberately: a clamp would leave client and
    compositor disagreeing about the pool's size, so the client would go on
    placing buffers at offsets it believes are inside its own mapping and
    collect confusing `invalid offset` errors from `create_buffer` later,
    somewhere other than the request that was wrong. Each refusal uses the code
    upstream itself uses for a bad size on *that* request — `InvalidStride` on
    `wl_shm` for `create_pool`, `InvalidFd` on `wl_shm_pool` for `resize` — so
    a client sees one consistent code whichever side decided.

    Refusing `create_pool` means returning without initializing the
    `New<WlShmPool>` it carries, and an uninitialized object's
    `UninitObjectData::request` is a `panic!`. That is safe for a specific
    reason, checked in wayland-backend 0.3.17 rather than assumed and recorded
    in the module doc so it needn't be re-derived: `post_error` calls `kill`
    synchronously, and `Client::next_request` returns `EPIPE` once `killed` is
    set, so no later request from that client is dispatched — *including one
    already buffered in the same `write()`*. `UninitObjectData::destroyed` is an
    empty no-op (so the `cleanup`/`queue_all_destructors` pass over that object
    does nothing) and `wayland-server`'s `New` has no `Drop` impl. A test
    pipelines `create_pool(i32::MAX)` + `resize(4096)` in one write for exactly
    this case.

    Bug-bashed on the dev VM against release binaries both ways, ten scenarios
    each (one under the cap, exactly at it, one over, and `i32::MAX`, for both
    requests; the pipelined pair; eight oversized pools in one batch; three
    oversized attempts from three fresh connections; an ordinary 4096-byte pool
    afterwards). After: in-range sizes accepted (`VmPeak` 677,168 kB confirms
    the 512 MiB mapping is really made), everything over refused with the
    message naming both numbers, the compositor alive and answering IPC
    throughout, the innocent client still served. Before: *every* oversized
    request accepted, and the eight-pool batch took the compositor's `VmPeak`
    to **16,864,580 kB (~16.1 GiB)** of reserved address space from one client
    in one batch — which is the finding, stated in numbers.

    **Tests: 24 new against the merge base** -- 22 in `-p flexwm` (200 total,
    from 178) and 2 in `-p flexwm-core` (45 total, from 43). The clamps
    are pure functions tested one under each bound, at it, and one over; the
    pool cap and the `min_size` read site are driven through a real
    `wayland-client` connection and a real `State`, the pattern
    `dispatch/tests.rs` established (extended here so the offending client is a
    closure, which is what let `create_pool` and `resize` share one harness).
    `shell/tests.rs` and `state/tests.rs` are new modules, split out rather
    than appended to their parents. Three **negative controls**, each run and
    recorded: reverting the gap clamp fails both new core tests (one with
    `attempt to multiply with overflow` inside `Rect::inset`); making
    `clamp_hint` the identity fails 6 shell tests (two with `attempt to add
    with overflow` inside `arrange.rs` — the finding in (b)); and disabling the
    `create_pool` guard fails the two tests that cover it, one with "the pool
    size was accepted" and one showing the pipelined `resize` then really does
    reach the pool object.

    **Benchmarked, because the pool cap adds a second `TypeId` check to the
    per-request path** — but the real proof `flexwm-reviewer` found is in the
    object code, not the jiffies: built the release profile with
    `strip=false` and counted `DisplayHandle::post_error`'s monomorphizations.
    It exists for exactly **3** concrete types: `WlShmPool` (this guard's
    resize check), `WlShm` (this guard's `create_pool` check), and
    `XdgWmBase` (Smithay's own, unrelated). If the `TypeId` comparison this
    guard adds had *not* folded away after monomorphization, that call would
    have been instantiated for every interface flexwm dispatches at all
    (`WlSurface`, `WlPointer`, `WlKeyboard`, `XdgSurface`, `XdgToplevel`,
    `WlBuffer`, ...) — it isn't. The guard body is compiled out of every
    interface's dispatch path except the two it actually checks, so a
    regression on an unrelated request type (like the `wl_surface.damage`
    flood this item benchmarks) was never possible in the first place. This
    is the finding; the jiffies below are corroboration, not the proof.

    Jiffies, for the record (same method as item 7: 1M `wl_surface.damage`
    requests, compositor `utime+stime`, one discarded warm-up rep, one
    discarded run of 16 balanced interleaved reps each of `a7ffef3` before
    and this commit after): before mean 36.19 (sd 4.21), after mean 35.69 (sd
    2.44) — a difference of 0.41 standard errors, indistinguishable from
    noise, nominally in the *faster* direction. An earlier, smaller two-binary
    run (12 reps) had shown before 35.33 vs after 38.50 and looked like a real
    ~9% regression; it was not reproducible at 16 reps and could not have been
    the guard regardless, once the object-code argument above is the actual
    basis for the conclusion rather than this measurement. Recorded as a
    caution: on this VM, a benchmark below roughly 16 balanced reps is not
    powerful enough to trust for a single-digit-percent effect, and jiffies
    alone should not be the only evidence for a hot-path change when a
    compile-time argument is available instead.

    Also verified at the same commit: `cargo test -p flexwm` 200/200 and
    `-p flexwm-core` 45/45 on the dev VM, clippy `-D warnings` clean for both
    crates there and `--workspace --all-targets` clean on macOS, `cargo fmt
    --all --check` clean on both hosts, `cargo check --workspace` clean on
    macOS (the cross-platform build), and `scripts/smoke-test.sh` green under
    `--headless` (11 `ok:` checks, exit 0) — which exercises the ordinary
    `wl_shm` buffer path through the new dispatch with a real `foot` window.

    **Found while bug-bashing, deliberately not fixed here** (see the new
    Backlog entry): `--width`/`--height` are raw unbounded `i32`s, so an
    operator's own `--width 2000000000` can still overflow the same
    `x + width` in `arrange.rs` that (b) closes for client-declared minimums.
    Out of this ticket's scope, and a CLI flag rather than a client- or
    config-supplied value, but it is the same family.

13. ~~A config override for the fallback cursor's size and color~~ — DONE,
    PR #19. Piece (b) of the Backlog's "Custom/client cursor support" entry,
    **for size and color only** — theme name stays open and is explicitly
    *not* what this shipped; see that entry, which now says so. Ahead of item
    6 for the same reason items 7-12 were: small and self-contained.

    `[appearance]` gained `cursor_size` (integer, default `16`) and
    `cursor_color` (`"#rrggbb"`/`"#rrggbbaa"`, default `#ffffff`), resolved
    through the existing `AppearanceConfig`/`into_appearance` path — so a
    malformed color degrades to that one field's default with a warning,
    exactly like the three ring/background colors, rather than invalidating
    `[appearance]`. `cursor.rs`'s `const SIZE` and its hardcoded
    white/black pixels are gone: `generate_bitmap(size, fill, outline)` is
    still pure and still returns the same bytes for the defaults, and
    `Cursor::default()` became `Cursor::new(size, color)`, called once from
    `State::new`. Built once at startup, never rebuilt — nothing in this
    project reloads config, and this ticket deliberately did not invent a path
    for it.

    **No theme name, by decision, not omission.** Honoring
    `CursorImageStatus::Named` properly needs a real xcursor asset or real
    xcursor-file loading; niri's assets are GPL and Adwaita's aren't MIT-clean
    (`CLAUDE.md`), so there is nothing in this repo to load and sourcing one is
    its own concern. `cursor.rs`'s module doc now says that at the point
    someone would look for the option.

    Three decisions worth the reviewer's attention:

    - **The outline is black at the fill's own alpha, not a second config
      field.** Its job is to keep the shape legible against similar-colored
      content, which a configurable outline could only undo; at the default
      opaque fill it is byte-identical to the fixed black outline this shape
      always had, and matching the alpha is what stops a translucent
      `cursor_color` from rendering as a solid black triangle outline around a
      see-through middle.
    - **`MIN_CURSOR_SIZE = 4`, `MAX_CURSOR_SIZE = 256`, justified
      arithmetically** the way `Config::MAX_GAP` is, not by taste. Below 4 the
      shape has no interior pixels at all (the outline owns the left column and
      the diagonal, so the fill only exists where `0 < x < y`: one pixel at
      size 3, none below). The upper bound is about the allocation, not looks:
      the bitmap is `size * size * 4` bytes, which is 256 KiB at the cap but
      **overflows `i32` outright at `i32::MAX`** — a debug panic inside
      `generate_bitmap`, a wrapped length in release, and the same product
      appears again in Smithay's own `assert!(mem.len() >= stride * size.h)`
      in `MemoryBuffer::from_slice`. `Appearance::clamp_cursor_size` is the
      pure, separately-tested clamp (`Config::clamp_gap`'s shape);
      `Appearance::clamped` applies it and warns, and `Cursor::new` applies it
      again at the allocation itself, the way `ring_rects` re-checks
      `width <= 0` rather than trusting its caller.
    - **`Color::to_argb8888` is a new single boundary for straight-alpha →
      premultiplied BGRA bytes**, next to the existing `From<Color> for
      Color32F` that does the same job for solid-color elements. Premultiplied
      because the pinned rev hands an `Argb8888` memory buffer to pixman as
      `a8r8g8b8` and composites with `Operation::Over`
      (`backend/renderer/pixman/mod.rs:389,605`), which is defined over
      premultiplied components.

    **The test gap this closed, found while writing the tests rather than
    after**: every pre-existing assertion about the cursor bitmap used white,
    black or the clear color, and **white and black are symmetric under a
    B↔R swap**, so nothing in the suite could have caught a byte-order
    mistake in a color conversion. The new live read-back test uses `#ff8040`
    (all three channels distinct) and deliberately *not* `#ff8000`, whose
    reversed bytes are this harness's own `CLEAR_BGRA` exactly — a swap would
    then draw the cursor in the background's color and the failure could not
    tell "wrong color" from "nothing drawn".

    **Tests: 20 new (221 total, against 201 on the merge base).**
    `Color::to_argb8888`'s
    byte order/premultiplication/saturation, the clamp at both bounds and at
    `i32::MIN`/`i32::MAX`, `generate_bitmap` at a non-default size and with
    non-default colors, a check that the defaults still describe the original
    16x16 white-on-black shape, `Cursor::new`'s re-clamp measured through the
    built element's real geometry, the config-file round trip (both fields
    set, only the cursor fields set, a malformed color, an out-of-range size,
    a size outside `i32`, and a gap small enough to clamp the ring but not the
    cursor), and two live read-back tests through a real `State` and a real
    `PixmanRenderer` — a 48px `#ff8040` cursor sampled at its outline,
    diagonal, interior, last filled row, the row past it and outside the
    triangle, plus a translucent `#ffffff80` one proving it blends with what
    is behind it instead of replacing it.

    Each of the three new live/clamp tests was confirmed non-vacuous against
    two negative controls (raw output in PR #19): `Cursor::new` stubbed to
    ignore the config entirely (all three fail), and `Color::to_argb8888`
    returning `[R, G, B, A]` instead of `[B, G, R, A]` (the live pixel test
    and the three byte-order unit tests fail, the rest pass — which is the
    exact blind spot described above).

    **Hardware verification** (dev VM, real `--tty` on its `virtio-gpu` KMS
    device at 1600x1000, debug build — i.e. with integer overflow checks on —
    all against `34514a9`, which is the whole of this item's executable code:
    every later commit on the branch changes only prose and doc comments, which
    `git diff 34514a9..HEAD -- '*.rs' | grep -E '^[-+]' | grep -vE '^(\+\+\+|---)' \
    | grep -vE '^[-+]\s*(///|//!|//)'`
    returning nothing confirms mechanically (the shorter, `grep -v`-only form
    a first draft of this note used does *not* actually confirm this — it
    still emits context lines and hunk headers, so seeing output from it is
    not a sign the cache key is stale, only a sign of using the wrong
    command; `flexwm-reviewer` caught this while reviewing item 13) — so the
    evidence key still matches. Six scenarios, each a fresh compositor with its
    own config, the pointer parked over the background by IPC and the frame
    captured by IPC; exact commands and every raw sampled pixel are in PR
    #19's description. Summary: `cursor_size = 48` + `cursor_color =
    "#ff8040"` draws `rgb(255,128,64)` at its interior and black at its
    hotspot/left edge/diagonal, with the last filled row at `+47` and
    background at `+48`; the same binary with no cursor fields draws the
    original 16x16 white shape with its boundary at `+15`/`+16`;
    `cursor_size = 100000` logs the clamp warning and draws a 256px shape
    (fill at `+255`, background at `+256`); `cursor_size = 0` logs it too and
    draws a 4px one (its single interior pixel at `(1,3)`, background at
    `+4`); `cursor_color = "not-a-color"` logs the per-field warning, draws
    white, and still honors `cursor_size = 32`; and `#ffffff80` over a
    `#203040` background reads `rgb(144,152,160)` at the fill and
    `rgb(16,24,32)` at the outline — both exactly halfway, which is what makes
    the premultiplication right rather than merely non-crashing.

    Edge cases on the same hardware, same build: a 256px cursor with its
    hotspot on the output's very last pixel `(1599,999)` draws its outline
    there and nothing is cut wrong; at `(1500,900)` it is clipped against both
    edges and still draws its interior and the diagonal that reaches the
    corner; clipped against the right edge alone it fills out to column 1599.
    Six adversarial absolute pointer positions (`-1`, `-100000`, `1e300`,
    `-1e300`, `2147483647`, and a fractional `1599.9 999.9`) each return `Ok`,
    each still capture a frame, leave the compositor alive with no panic or
    error in its log, and an ordinary position still draws afterwards.

    **Caveat on these logs, worth recording precisely rather than glossing
    over**: every one of these runs, over SSH, logged smithay's own `Unable
    to become drm master, assuming unprivileged mode` at device-open time —
    `flexwm-reviewer` reproduced this independently and confirmed it isn't
    specific to this item. This doesn't touch any claim above (the composited
    frame is read back and pixel-sampled the same way regardless of DRM
    master state, so the cursor-rendering claims hold either way), but it's
    in real tension with `vm/README.md`'s claim that a `--tty` session over
    SSH "takes real DRM/libseat ownership just fine." Nothing here confirms
    or refutes whether master gets acquired later via libseat, only that it
    isn't held at open time — an open question for whoever next needs to
    trust a claim in this project that specifically depends on holding real
    DRM master (VT-switch/scanout behavior, not pixel content), not
    something this item's own evidence needed to resolve.
    **Resolved 2026-09-13** — see the resolved DRM-master entry in the Backlog
    below. Master *is* held, over SSH and from a real VT alike; the warning
    means "this process may not call `SET_MASTER` itself", and the fd seatd
    passes is already the master. One correction to the framing above, for the
    record: master isn't acquired later by flexwm *making a successful call*
    either — flexwm never itself issues a working `SET_MASTER` at all (its own
    attempt always gets `EACCES`, by design, see the Backlog entry). It's
    seatd's open-and-set that establishes it, and seatd's own pause/activate
    logic that releases and re-establishes it across a VT switch (confirmed
    live: master genuinely toggles `y → n → y` in step with a `chvt` cycle,
    not held uninterrupted the whole time the process runs). `vm/README.md`'s
    conclusion was right and only its stated mechanism (logind/PAM) was wrong;
    both are fixed.

    The item-8 client-cursor behavior is unchanged and is covered by its seven
    tests passing untouched (they drive a real client and assert read-back
    pixels, so "the client's image still wins over the configured fallback" is
    a real assertion, not an inference): `element()` returns before it ever
    touches the fallback buffer when a live cursor surface exists, and no
    state is shared between the two paths.

    **Benchmarked** (same jiffies-delta method as items 5/8; release builds of
    `8446758` (base) and `34514a9` (new), three arms interleaved per rep so VM
    drift hits all of them). The change cannot cost anything per frame in
    principle — the bitmap is built once at startup and `element()` is
    untouched — but a *bigger* bitmap composites more pixels every frame it is
    drawn, so the cap's own cost was measured too, not just the default's.
    200 pointer moves per rep, 6 reps:

    | arm | base (16px) | new (16px) | new, `cursor_size = 256` |
    |---|---|---|---|
    | far: `(0,0)`↔`(1344,744)` | 29.67 (25-36) | 30.50 (27-36) | **42.00 (36-54)** |
    | near: `(4,4)`↔`(8,8)` | 8.83 (7-11) | 9.50 (8-12) | **14.17 (12-16)** |

    The default is unchanged either way (overlapping ranges, base vs new) —
    also true analytically, not just by measurement: no per-frame allocation
    was added, `element()` is untouched, and the persistent render buffer and
    stable element `Id` are preserved. The largest cursor a config can ask for
    costs *something* real: `flexwm-reviewer`'s own re-run (fixed arm order,
    unrotated, so drift can load onto whichever arm runs last) found one
    `new256` sample at 17 — inside `new`'s own range and below every reported
    `new256` value — so the specific "+12 far / +5 near jiffies" figures above
    are order-of-magnitude, not a tight measured delta; read them as "a
    256px cursor costs single-digit-to-low-double-digit jiffies more per 200
    moves," not as exact numbers. The direction and the reason (more pixels
    composited per frame) are solid regardless, and bounded by
    `MAX_CURSOR_SIZE`, which is the practical argument for having an upper
    bound at all beyond the overflow one.

    **A correction worth recording, because the first version of this
    measurement was a no-op.** It drove the pointer with
    `pointer move 100000 100000` / `-100000 -100000`, on the assumption that
    IPC's `pointer move` takes a relative delta. It does not — it is absolute
    and is *not* clamped to the output (that clamp belongs to
    `pointer_move_relative`, libinput's path), so both endpoints were
    off-screen, no cursor was composited in any arm, and all three came out
    identical at ~21 jiffies. The numbers above use endpoints chosen so that
    even a 256px shape is fully on screen at both (1344 + 255 = 1599 on a
    1600x1000 output), and the script now screenshots both endpoints first and
    prints the hotspot, an interior pixel and the pixel 255 rows down as a
    witness that the thing being measured is actually being drawn.

    Also at the same commit: `cargo test -p flexwm` 221/221 and
    `-p flexwm-core` 45/45 on the dev VM, clippy `--workspace --all-targets
    -D warnings` and `cargo fmt --all --check` clean on both the VM and
    macOS, `cargo check --workspace --all-targets` clean on macOS (the
    cross-platform build), `scripts/smoke-test.sh` green under `--headless`
    (all 11 `ok:` checks, exit 0) against a release build of this branch, and
    the README's example `config.toml` — now including the two new fields —
    loaded by a real compositor with no warning or error in its log.

14. ~~`wlr-layer-shell-unstable-v1`: bars, docks, wallpapers and notification
    daemons~~ — DONE, PR #22. Picked up from the Backlog (entry struck
    below), explicitly ahead of item 6 at the user's request: before this,
    *no* panel, launcher or notification daemon could attach a surface to
    flexwm at all, which is the single biggest gap between "works" and
    "daily-drivable."

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

## Backlog (unordered — pick up whenever it fits)

- **~~Open question: does `--tty` over SSH on the dev VM actually hold real
  DRM master, or is it running in "unprivileged mode" the whole time?~~ —
  RESOLVED 2026-09-13 (investigation + docs fix, PR #20). It holds real DRM
  master. The warning is a red herring, it is not SSH-specific, and no
  flexwm code change is warranted.** Raised by `flexwm-reviewer` while
  reviewing item 13: every `--tty` run over SSH logs Smithay's own `Unable to
  become drm master, assuming unprivileged mode` at device-open time, in
  apparent tension with `vm/README.md`. Answer, from primary sources plus
  live measurement on the dev VM at `0678765`:

  **Mechanism.** "Unprivileged mode" is Smithay's name for "this process may
  not call `SET_MASTER` itself", not "this process is not master". Master
  goes to whichever open file is *first* to open the device while nothing
  else already holds it, plus seatd's own explicit `DRM_IOCTL_SET_MASTER`
  call on that file right after (`seatd/seat.c`) — root is not what grants
  master (`drm_master_open`, `drivers/gpu/drm/drm_auth.c`, hands it to the
  first opener regardless of uid, and root can't take it from an existing
  holder either: `drm_setmaster_ioctl` returns `EBUSY`); root is what lets
  seatd open the device node and manage VTs at all. Since seatd is normally
  the only thing that ever opens the GPU node on this VM, in practice it *is*
  first, and passes that already-master fd over its socket to flexwm, which
  inherits it (master is a property of the open file, not the process) — but
  that is a fact about this VM's setup, not something "opened by seatd"
  guarantees in general, and the caveat below is exactly why. flexwm's own
  `SET_MASTER` inside Smithay's `DrmDeviceFd::new` is then refused with
  `EACCES` because kernel 6.18's `drm_master_check_perm`
  (`drivers/gpu/drm/drm_auth.c`) requires `was_master && file->pid ==
  current->tgid` (or `CAP_SYS_ADMIN`), and `drm_file_update_pid`
  (`drm_file.c`) deliberately never re-owns a file that was master — so the
  fd's recorded owner stays seatd forever. Smithay's resulting `privileged =
  false` is the *correct* state for the libseat path: it is what stops
  `DrmDevice::pause`/`activate` (`device/mod.rs:417`/`431`) from issuing
  `SET_MASTER`/`DROP_MASTER` themselves, which seatd already does as root on
  every VT switch. **The identical warning also fires when master genuinely
  isn't held**: if something else already has it when seatd opens the device,
  seatd's own `SET_MASTER` gets `EBUSY` too, only logs it, and hands the fd
  over anyway — that case fails loudly at modeset instead of at open, which is
  exactly why the evidence below checks the actual kernel state rather than
  trusting the log line alone.

  **Evidence** (commands and raw output in PR #20's description). While an
  SSH-started `--tty` runs: `/sys/kernel/debug/dri/0/clients` shows exactly one
  client, `seatd 460 ... master y`; a root `drmSetMaster` probe gets `EBUSY`
  (root cannot take master, i.e. someone holds it) and `drmIsMaster` reads 0;
  `/sys/kernel/debug/dri/0/state` shows `crtc-0 enable=1 active=1`, mode
  `1600x1000`, with the plane's `fb=42` *allocated by flexwm* — real scanout,
  not the pixman intermediate. A `chvt 2`/`chvt 1` cycle moves all of it in
  lockstep: `master n` + plane back to `[fbcon]`'s fb + probe acquires master
  freely while paused, then `master y` + plane back to flexwm's fb + `EBUSY`
  again after the switch back. The `EACCES`-despite-master condition was also
  reproduced in isolation with no seatd involved at all (a process opens
  card0, `drmIsMaster=1`; its forked child, same open file, different tgid,
  also reads `drmIsMaster=1` but gets `EACCES` from `drmSetMaster`) — which is
  what makes "the warning does not mean what it looks like" a fact rather than
  an inference. The same warning appears verbatim when started from a real VT
  (`openvt -c 3 -s`, tty3 as controlling terminal and foreground VT) with
  master equally held, so it was never about SSH.

  **Why SSH works at all — `vm/README.md`'s stated reason was wrong, its
  conclusion was right.** It credited "logind's PAM stack registers those with
  a real seat/session too." It doesn't: `loginctl` reports `Seat=`, `VTNr=0`,
  `Remote=yes` for an SSH session, and `LIBSEAT_BACKEND=logind flexwm --tty`
  over SSH fails immediately with `Failed to open session: No data available`.
  What actually happens is that libseat uses its **seatd** backend, and
  seatd's `seat0` is VT-bound: `seat_add_client` assigns every client
  `seat->cur_vt`, the VT in the foreground at connect time, regardless of how
  the process was started. Confirmed by the session number in seatd's own log
  tracking the foreground VT: `Added client 1 to seat0` over SSH with tty1
  foreground, `Added client 3 to seat0` for the `openvt -c 3 -s` run. Fixed in
  `vm/README.md` (mechanism, a "is it really DRM master" troubleshooting entry
  with the two debugfs checks, and the two consequences of VT binding: one
  libseat client at a time on a VT-bound seat, and `XDG_RUNTIME_DIR` must
  exist) and in `vm/configuration.nix`'s `services.seatd.enable` comment,
  which carried the same wrong claim.

  **What this means for prior hardware claims: they stand, and now have a
  mechanism behind them.** Items 3 and 5b are the ones that specifically
  depended on *holding* master (real scanout; VT-switch pause/reactivate
  semantics), and the cycle above re-demonstrates both directly. The
  historical runs are covered too, without re-running them: seatd logs
  `Could not make device fd drm master: ...` whenever its own root-side
  `drm_set_master` fails, and
  `journalctl -u seatd | grep -c 'Could not make device fd drm master'`
  returns **0** across this VM's entire persistent journal — 2072 seatd lines
  and 323 `Opened client` events, reaching back to its first boot at
  2026-09-11 15:16:57 — while those same runs logged `drm: modeset (full
  commit)` with no error, and `DRM_IOCTL_MODE_ATOMIC` is `DRM_MASTER`-gated by
  `drm_ioctl.c`, so the kernel would have returned `EACCES` had master not
  been held. One limit, stated rather than papered over: nothing here is a
  *photograph* of the QEMU window — host `screencapture` is blocked by macOS
  Screen Recording permission in this environment ("could not create image
  from display"). The scanout evidence is the kernel's own atomic state plus
  the master-gating of the ioctl that set it, which is strictly more specific
  than a photo, but if a future claim wants a visual, look at the window.

- **Input injection targeted at a specific window, without moving seat
  focus (research-backed idea, 2026-09-12 — a major goal per `CLAUDE.md`,
  not automatically ahead of daily-drivability items like layer-shell; see
  that file's current framing before assuming either wins by default).**
  The single most concrete gap identified while researching related
  projects: an agent
  driving a real desktop needs to act on a window that is not the one
  currently focused, without stealing focus away from whatever a human (or
  another agent) has in the foreground. A recent deep-dive on Linux
  computer-use automation names this as one of the hardest unsolved
  problems in the space — every existing tool hacks around it per-toolkit
  (different tricks for GTK3/GTK4/Qt/Electron) because no compositor
  exposes a clean primitive for it. A real automation project hit exactly
  this wall trying to use niri: screenshot capture worked, but every input
  action was refused, with the literal error "no trusted Wayland compositor
  adapter can confirm exact target window" — niri's own IPC can identify
  and activate a specific window, but nothing lets a caller inject input
  *into* one without first making it the real focused window.

  **flexwm is unusually well-positioned to solve this properly**, since it
  owns the entire Wayland dispatch stack itself rather than automation
  being bolted onto a desktop environment that wasn't built for it, and
  half the identity/targeting problem is already solved:
  `Request::Action(FocusWindowId(id))` already proves by-ID window
  targeting exists in the IPC surface today. What's missing is a sibling
  request that dispatches input at a window ID *without* calling that real
  focus-changing path.

  **Why this isn't a trivial tweak, so scope it honestly.** Wayland's
  keyboard/pointer protocols are focus-gated at the client level — a client
  only processes key/pointer events after a matching `enter`, and most
  toolkits track "am I focused" from that sequence, not the raw events
  alone. So the real mechanism is not "inject with literally zero focus
  interaction" — the same research above notes real implementations often
  still need "synthetic focus events... without raising the app." The
  actual design question is how to give the *target* surface a scoped
  synthetic enter/key(or button)/leave sequence without touching the seat's
  real global focus state (`Seat`/`KeyboardHandle`/`PointerHandle` in
  Smithay) or visibly raising/activating that window — i.e. the target
  briefly believes it is focused for exactly the injected event, while
  every other client and the compositor's own idea of "what's actually
  focused" is unaffected. This needs real investigation into what the
  pinned Smithay revision's `KeyboardHandle`/`PointerHandle` actually expose
  for this (a lower-level per-surface send path bypassing the seat's single
  held-focus abstraction, if one exists) before committing to a design —
  don't assume the API shape without checking, per this project's standing
  rule on Smithay claims.

  **Open question, not yet answered:** what does this mean for a window
  that is not currently visible at all (scrolled off-screen in the
  horizontally-scrolling layout, or on a different workspace/output)? Does
  this project's render loop already process commits/frame-callbacks for
  mapped-but-not-currently-visible windows (needed either way, independent
  of this feature), or would that need its own fix first? Investigate
  before scoping an implementation.

  Concrete shape once designed: likely a new `Request` variant (naming
  TBD — something like a `window` field added to the existing `Key`/
  `Click`/`PointerButton` requests, or dedicated variants) in `flexwm-ipc`,
  implemented in `input.rs` alongside the existing focus-changing
  dispatch it must *not* reuse.

- **Enhanced testing ideas for the hardware/DRM-dependent gap (research,
  2026-09-12) — evaluate, none committed to yet.** This project's `--tty`
  backend (real DRM/libseat/libinput) has essentially zero unit-test
  coverage by necessity — `Tty` holds real kernel handles a test process
  can't construct — so its correctness today rests entirely on code-reading
  plus manual hardware bug-bash on the dev VM before every merge. Researched
  how other projects close a version of this same gap:
  - **VKMS (`vkms.ko`), a real in-kernel virtual DRM/KMS driver purpose-built
    for this.** Gives a genuine `/dev/dri/cardN` with an emulated CRTC/
    connector/plane, no physical GPU needed. Mesa's own CI combines VKMS +
    llvmpipe (software rendering) + a real Wayland compositor for headless
    DRM-path testing; Collabora published a concrete recipe for testing
    Weston's actual DRM backend this way using `virtme` (a QEMU wrapper
    that boots a custom kernel sharing the host's rootfs, lighter than a
    full VM image). Their own stated caveat: "VKMS does not substitute a
    real graphics card yet," device enumeration order isn't deterministic,
    and they still test on real hardware too — a complement to hardware
    bug-bash, not a replacement for it. This project's dev VM is already a
    real (if virtual, via virtio-gpu) DRM device for manual/agent-driven
    testing; VKMS would be the piece that makes some slice of that
    *automated and CI-runnable* instead of always requiring a live VM
    session. Non-trivial setup cost (custom kernel config, `virtme`,
    figuring out what's actually exercisable without display output).
  - **Property-based testing for `flexwm-core` specifically (the cheapest,
    most directly actionable idea here, no new infrastructure needed).**
    `flexwm-core` is already pure and I/O-free by design (no Wayland, no
    I/O — see this file's vision note), exactly the shape property testing
    wants: generate random sequences of window-management actions (via the
    `proptest` crate) and assert invariants hold (no window ever gets a
    negative width, focus always points at a window that still exists, a
    workspace's columns stay internally consistent after any action
    sequence) instead of hand-writing every case. Confirmed niri — the
    project this one is explicitly modeled on — does exactly this for its
    own layout logic (per its `CONTRIBUTING.md`: "for new layout actions,
    we add randomized tests"), alongside "client-server tests" for Wayland
    protocol edge cases — the same two-tier split (pure unit/property tests
    + live client-server integration tests) this project has already
    organically converged on via `dispatch/tests.rs`/`ipc/tests.rs`.
  - **WLCS (Wayland Conformance Test Suite)**, a shared protocol-level
    black-box suite any compositor can plug into via a small adapter —
    Smithay's own reference compositor does this (`wlcs_anvil`). Tests real
    client-visible protocol behavior without touching internals; a bigger
    lift to adopt than the other two, but conceptually the same idea as
    this project's own "drive it over IPC/the protocol and check what
    happens" tests, standardized and shared across compositors.
  - **libinput's own approach for input hardware** (`litest`): builds
    virtual devices via the kernel's `uinput` driver to exercise real
    input-handling code without physical hardware — this project already
    does something in the same spirit (a real `/dev/uinput`-injected
    keystroke was part of item 3's original hardware verification).
  Rough priority if picked up: property-based tests for `flexwm-core` first
  (cheap, zero new infrastructure, closes a real gap immediately);
  VKMS-in-CI second (bigger payoff — real automated DRM-path testing
  instead of always needing a live VM session — but real setup cost); WLCS
  as a longer-term, larger investment worth knowing exists.

- **~~`State::listen` panics (aborts the process) if `$XDG_RUNTIME_DIR` is
  unset, instead of a clean startup error (LOW)~~ — DONE as item 12(a)**, with
  the before/after release-binary evidence (core dump vs one-line error)
  recorded there. Original diagnosis, left as written: found incidentally by
  `flexwm-reviewer` while reviewing PR #14, unrelated to that PR and left
  untouched by it. `crates/flexwm/src/compositor/state.rs:196`:
  `ListeningSocketSource::new_auto().expect("a free wayland socket")` — the
  actual failure in this case is Smithay's `RuntimeDirNotSet`, which
  `.expect()` turns into a panic and a core dump rather than a message
  telling the operator what's actually wrong. Fix: match on the error and
  print a clear startup error instead of panicking, same shape as this
  project's other clean-startup-error paths (e.g. `ipc::init`'s "no socket
  path" error).

- **~~`wlr-layer-shell-unstable-v1` protocol support~~ — DONE as item 14**,
  except for layer popups (the entry two below). The original entry's guess about the shape turned out half
  right: the `Elements` enum did need changing, but into *one* `Surface`
  variant rather than a fourth, and the exclusive-zone plumbing into
  `flexwm-core` was one new field plus one new event, not a restructure.

- **~~Layer-shell keyboard interactivity (`keyboard_interactivity`)~~ — DONE,
  folded back into item 14** after review pushed back on shipping the gap.
  The model this entry laid out (exclusive on top/overlay takes focus while
  mapped and front-most wins; `on_demand` anywhere and `exclusive` on
  bottom/background are click-to-focus; `none` never) is what was built,
  unchanged. Two things the entry did not anticipate: keyboard focus is
  *derived* from the layer map on every refresh rather than stored as an
  override `apply()` respects, which removed most of the teardown surface it
  worried about; and "while it is mapped" needed to mean "has a buffer"
  (`LayerSurfaceCachedState::last_acked`), not "is in the layer map", or a
  surface that draws nothing could hold every keystroke. See item 14.

- **No `xdg_popup` ever receives its initial configure, so no popup maps at
  all** (found while implementing item 14; pre-existing and unrelated to
  layer shell). `handlers.rs`'s `new_popup` tracks the popup in
  `PopupManager` and `commit` calls `PopupManager::commit`, but nothing
  calls `PopupSurface::send_configure` — and the pinned rev's
  `PopupManager::commit` only moves a popup from unmapped to mapped, it does
  not configure (checked: `desktop/wayland/popup/manager.rs:38-52`). Anvil
  does this in its own `ensure_initial_configure`. Consequence: a client
  menu, dropdown or tooltip never appears, from a window *or* a layer
  surface — which is why item 14 deliberately does not implement
  `WlrLayerShellHandler::new_popup` either: tracking a popup that can never
  map would be dead code. Fix is small (configure on first commit, the same
  shape `send_initial_configure` already has for toplevels) but wants its
  own tests, since it makes popups appear for the first time and nothing in
  the render path has ever drawn one.

  Reproduced, not just traced: `no_xdg_popup_is_configured_yet` (in
  `layer_shell/tests.rs`) has a real client create an `xdg_popup` on a
  mapped toplevel with a valid positioner and round-trip ten times — no
  `xdg_surface.configure` ever arrives, and the compositor stays up. Turning
  that assertion around is what the fix should do; delete the test and this
  entry together.

- **A layer surface that commits but never attaches a buffer holds its
  exclusive zone** (item 14, deliberate, documented in
  `a_bar_reserves_its_zone_from_its_initial_commit_not_its_first_buffer`).
  Smithay's `LayerMap::arrange` arranges every surface mapped into the map,
  buffer or not, so the reservation starts at the initial (buffer-less)
  commit the protocol requires. For a healthy bar that window is a frame or
  two; for a client that commits and then hangs, the space stays reserved
  until it disconnects. Fixing it means filtering on mapped-ness while
  computing the zone, which today would mean reimplementing `arrange`
  locally — worth doing only if a real client is seen to hit it, or if
  upstream grows the distinction.

- **~~`flexwm msg type` silently drops every shifted character, so an agent
  cannot type a capital letter (MEDIUM, and squarely against the
  computer-use goal).~~ — RESOLVED 2026-09-13 (fix + tests, PR #23).**
  `needs_shift` is gone. `crates/flexwm/src/compositor/input/modifiers.rs`
  answers both halves of the question in one keymap walk: which key carries
  the keysym *and* which level it sits at (the old code asked two different
  Smithay helpers and only one of them looked past level 0). It then asks
  `xkb_keymap_key_get_mods_for_level` which modifier combinations reach that
  level and holds the keys for the cheapest one it can actually produce.
  *Which* keys those are also comes out of the keymap rather than a
  modifier-name-to-keysym table: every keycode is pressed once in a
  throwaway `xkb::State` and watched, so AltGr levels work on layouts that
  put AltGr somewhere else (checked against `de` and `de(neo)`), a modifier
  key with no keysym of its own is still usable, and — the reason the probe
  earns its keep — a key that *locks* or *latches* its modifier (Caps Lock,
  Num Lock, `ISO_Level3_Latch`) is never pressed. `key_get_mods_for_level`
  really does offer Caps Lock as an alternative to Shift for a capital
  letter, and pressing it would type one capital and leave the keyboard
  shifted for everything typed afterwards. A level only such a key can reach
  is an error instead (`Untypable::NoModifiers`), as is a character no key
  carries at all (`Untypable::NoKey`) — both loud, where the old behaviour
  was silent and wrong. The resolution pass allocates nothing (fixed-size
  `Copy` results, and the keymap probe runs at most once per request, only
  once a character actually needs a modifier).

  **Cost, corrected** (an earlier draft of this entry claimed "one keymap
  scan per character instead of two", which the benchmark does not support
  and which review caught): the shapes are the same either way. The old
  path was one scan (`keycode_for_keysym`) plus one O(1) level-0 lookup;
  the new one is one scan (`key_for`) plus one O(1)
  `key_get_mods_for_level`. That is exactly why the benchmark shows
  lowercase typing unchanged within noise (median 114 ms → 112 ms per
  50,000 characters). The real cost is confined to characters that were
  previously typed *wrongly*: they now send four key events instead of two
  (the modifier's own press and release), which is the whole point, and it
  takes the all-shifted median from 108 ms to 188 ms per 50,000.

  **Verified** three ways, all red/green (the fix disabled, the tests fail
  with exactly the reported symptom; restored, they pass): 10 unit tests
  against real `us`/`de`/`de(neo)` keymaps
  (`input/modifiers/tests.rs`); 7 live-client tests that decode what a real
  `wayland-client` toplevel received through its own `xkb::State`, built
  from the keymap fd the compositor sent it (`input/tests.rs` — the
  "assert on what the client actually got" test the fix shape below asked
  for, in its own harness rather than by extending the layer-shell one);
  and a new `scripts/smoke-test.sh` step that types a shell command
  containing every broken character class into a real `foot`, redirects it
  to a file and diffs it byte for byte. The script itself ran on
  `--headless`; the same round trip, run by hand, ran on real `--tty`
  hardware in the dev VM (`/home/dev/tty-evidence.sh` there, screenshot
  artifact `/home/dev/tty-shift-evidence.png`) — byte-for-byte identical
  both times, `od -c` output in PR #23. Deliberately left out of scope and
  split out as its own entry below: dead keys, compose sequences, and
  characters that are only on a layout other than the active one.

  **`flexwm-reviewer` found a blocking bug in the fix itself, fixed in the
  same PR (second round).** The first version asked the keymap two
  questions in two different xkb *groups*: `ModifierKeys::probe` built a
  scratch `xkb::State`, which starts in group 0 and was never pinned, while
  `plan` resolved the character's level against the *active* group. Key
  actions are per-group exactly as keysyms are, so with `XKB_DEFAULT_LAYOUT=de,de
  XKB_DEFAULT_VARIANT=neo, XKB_DEFAULT_OPTIONS=grp:menu_toggle` and the
  session toggled into group 1, `flexwm msg type '@'` typed **`#q`** — the
  probe had recorded group 0's third-level key (`de(neo)`'s, in the `#`
  position), which is an ordinary `#` key in group 1, so the modifier was
  never set and `@`'s key fell through to `q`. Silent, and exactly the
  failure mode this whole entry exists to close, one level up. `probe` now
  takes the layout, pins the state to it (`update_mask`) before every key
  it presses, and discards a state that comes back with either the modifier
  state or the group changed; `ModifierKeys` carries the layout it answered
  for, so `plan`'s per-string cache can't hand a group-0 table to a group-1
  character. Every earlier test compiled a single-group keymap, where group
  0 *is* the active group, which is why this shipped unnoticed — so the
  regression tests are multi-group and assert on what a client decodes.
  Reproduced on the dev VM before the fix and re-run after
  (`/home/dev/fix-multilayout.sh`), and the real `--tty` round trip above
  was re-run against the corrected code rather than carried forward.

  The same review round found the identical bug class still live in
  `flexwm msg key` — see the entry below for what it did and what replaced
  it.

  **Where.** `input.rs`'s `needs_shift` decides whether to hold `Shift_L`
  around a character, and asks Smithay `xkb.raw_syms_for_key_in_layout(...)
  .contains(&keysym)`. That helper is hard-coded to **level 0**
  (`input/keyboard/mod.rs:187-189` in the pinned rev:
  `key_get_syms_by_level(keycode, layout.0, 0)`), i.e. it returns exactly
  the syms that need *no* modifier — so `contains` is false for every
  keysym that does need one, and `needs_shift` can only ever answer
  "false". `'A'`, `'!'`, `'_'`, `'?'`, `'~'`, `':'` and `'|'` all come out
  as their unshifted twin.

  **Why it is silent rather than an error.** `keycode_for_keysym`
  (`mod.rs:1240-1253`) scans *every* level, so the lookup for `'A'`
  succeeds and returns the `a` key; only the shift decision fails. That
  asymmetry between the two Smithay calls is the whole bug, and it is also
  why three rounds of hardware testing missed it: every test string anyone
  happened to type was lowercase.

  **Why it matters.** Driving a computer through `flexwm msg type` is a
  stated goal of this project, and an agent that cannot produce a capital
  letter cannot type a password, a `Dockerfile`, a URL with a query string,
  a shell pipeline, or most identifiers in most languages. It is a bigger
  practical hole in agent-driven use than anything currently above it in
  this backlog.

  **Fix shape.** Find the level the keysym actually sits at
  (`num_levels_for_key` + `key_get_syms_by_level`, the same walk
  `keycode_for_keysym` does) and press the modifiers that level needs —
  `xkb_keymap_key_get_mods_for_level` rather than an assumption that level
  1 means Shift, since AltGr levels exist on plenty of layouts and `press`
  already has a `Modifier` → keysym mapping to reuse. Wants a test that
  asserts on what a real client *received* (keysym plus modifier state),
  not just that keys arrived: the layer-shell harness is the only one with
  a real `wl_keyboard`, and it currently counts events without decoding
  them, so it needs extending first.

- **`flexwm msg type` still can't produce a character that needs a dead key,
  a compose sequence, or a layout the session isn't currently on (LOW).**
  The shifted-character fix above covers every character that is *on* the
  active layout at some shift level. That is *not* "all of ASCII on any
  Latin layout", as an earlier draft of this entry claimed (review caught
  it; the numbers below come from
  `every_planned_character_decodes_back_to_itself_on_every_latin_layout`
  and its sibling, which sweep printable ASCII across fourteen real
  layouts and record exactly what each refuses). Coverage is
  layout-dependent: `us`, `us(intl)`, `gb`, `de(neo)`, `fr`, `fr(oss)`,
  `it` and `pl` produce all 95 printable ASCII characters, while `de` and
  `es` refuse `^` and `` ` ``, and `pt`, `se`, `no` and `dk` refuse `~` as
  well — those keys carry `dead_circumflex`/`dead_grave`/`dead_tilde`, not
  the plain character. `~` is the one that bites in practice: shell paths,
  globs and regexes are full of it.

  Three things are still out of reach, all of them refused loudly rather
  than typed wrong — the first two with ``no key for `X` in this layout``,
  the third with its own message (``[X] needs a modifier this layout only
  locks or latches``, `input.rs`), since "your layout doesn't have this"
  and "your layout has it but only behind Caps Lock" call for different
  responses:
  - **Dead keys and compose sequences.** `é` on a plain `us` layout is two
    or three keypresses with a state machine in between (`Compose`, `'`,
    `e`), not a key with a level. Driving it needs an `xkb::Compose` table
    and a second resolution path when the single-key lookup fails; the
    payoff is accented Latin text an agent might paste, so this is worth
    doing if that ever comes up, not before.
  - **Characters on an inactive layout group.** Resolution uses the
    keymap's active layout only. With `us,de` configured, `ü` is one group
    switch away, and nothing here switches groups — deliberately: the
    session's own layout is user state, and silently changing it to type
    one character (or failing to change it back if the request errors
    partway) is worse than saying no.
  - **Levels only a locking or latching modifier reaches.** Refused on
    purpose, see the resolved entry above; a layout that puts a character
    *only* behind Caps Lock would need it pressed and un-pressed around the
    character, and nothing in xkbcommon promises that round-trips cleanly.

  Adjacent, and **not** unchanged — an earlier draft of this entry said
  `flexwm msg key A` typing `a` was `press`'s documented contract rather
  than the same bug, and review measured that it was the same bug, still
  live and wider than one name. `key exclam` typed `1`, `key at` typed `2`,
  and `asciitilde`, `underscore`, `question`, `colon`, `bar` and
  `braceleft` were all off by one level the same way, because
  `keycode_for_keysym` returns the lowest keycode carrying a keysym at
  *any* level while `press` holds only the modifiers its caller names.
  Fixed in the same PR: those names are now refused with a message saying
  what to write instead, `modifiers::named_key` resolves names by looking
  for a level-0 carrier, and `keycode_for` — the last caller of
  `keycode_for_keysym` — is gone. `flexwm msg key shift+1` types `!` and
  `shift+a` types `A`, as before.

  Note what that refusal does *not* buy: some characters cannot be named as
  a combination at all. `@` on a German layout needs AltGr, and `Modifier`
  has no name for it (`ctrl`/`shift`/`alt`/`super` only), so `msg type` is
  the only way to produce it. Giving `key` a name for the third-level
  modifier is a plausible small follow-up if an agent ever needs to chord
  with AltGr; nothing needs it today.

- **`flexwm msg outputs` reports only an output's full rectangle**, so an
  agent cannot see what a bar reserved (item 14 gave the core a `usable`
  area but did not extend the IPC surface). Adding a `usable` rect to
  `OutputSnapshot` is a one-field, version-bumping change to `flexwm-ipc`;
  it is worth doing alongside whatever else next changes that wire format
  rather than bumping `PROTOCOL_VERSION` on its own.
- **`ext-workspace-v1` protocol support.** `flexwm-core` already has a real
  workspace model (`Output::workspaces`, `active_workspace`,
  `FocusWorkspace`/`MoveWindowToWorkspace` actions in `world/mod.rs` and
  `world/actions.rs`) — it's just not exposed outside keybindings/IPC
  actions today. This protocol (the modern, compositor-agnostic successor
  to the various one-off wlr workspace protocols) would let external tools
  (bars, workspace switchers/indicators) query and switch workspaces the
  same way sway/Hyprland's bars do. Unlike layer-shell, the pinned Smithay
  rev has **no existing helper for this protocol at all** (checked: no
  `ext_workspace` reference anywhere in the pinned checkout) — the global,
  object lifecycle, and event plumbing would need to be implemented
  directly against `wayland-server`, not layered on a Smithay helper.
  Depends on layer-shell landing first in practice, since the main
  consumers (bars) need both to be useful together. No design work done
  yet.

- **`ext-session-lock-v1` protocol support — the highest-priority protocol
  gap, and directly tied to a real safety finding already on record in
  this file (item 14's third review round, the `keyboard_interactivity`
  discussion).** That review flagged that a layer-shell client trying to
  act as a screen locker (`gtklock`, `swaylock-effects`, pre-1.7
  `swaylock`) is inherently the wrong mechanism: it depends on
  `exclusive` keyboard interactivity, a layer surface can be closed or
  covered, and nothing stops another client from drawing over or under
  it or a screenshot tool from capturing what's behind it. `ext-session-
  lock-v1` exists specifically to replace that pattern: the compositor
  itself blanks every output, refuses input to anything but the lock
  client's own surfaces, and the client only regains normal behavior once
  it explicitly unlocks — a fundamentally different trust model than "a
  layer surface asked nicely for exclusive focus." User request,
  2026-09-13. No design work done yet; check whether the pinned Smithay
  rev has a helper for this one (unclear without checking — it's newer
  than layer-shell but has seen wider compositor adoption, so it may or
  may not be present the way `wlr_layer` was).

- **`ext-idle-notify-v1` and `idle-inhibit-unstable-v1` — pairs naturally
  with session-lock.** User request, 2026-09-13. `ext-idle-notify-v1` is
  what lets an external tool (a `swayidle`-style daemon) learn "the user
  has been idle N seconds" so it can dim the screen, lock it, or suspend
  the machine — without it, `ext-session-lock-v1` above has no automatic
  trigger, only a manual one. `idle-inhibit-unstable-v1` is the reverse:
  lets a client (a video player, a presentation app) tell the compositor
  not to consider the session idle while it's active. Neither has design
  work done; natural to scope alongside session-lock since they're the
  same feature area (idle/lock lifecycle), not before it.

- **Foreign-toplevel management (window enumeration for external tools).**
  User request, 2026-09-13. `ext-workspace-v1` above covers workspaces;
  nothing today gives an external client (a taskbar, an alt-tab switcher
  applet) the equivalent view of *windows* — only flexwm's own IPC
  (`flexwm msg windows`) has that. Check which protocol is actually
  current before implementing: `wlr-foreign-toplevel-management-unstable-v1`
  is the older, widely-supported one; there may be a newer `ext-` successor
  by the time this is picked up, and per `CLAUDE.md`'s standing preference
  for the compositor-agnostic successor where one exists, that should win
  if it does. No design work done.

- **`xdg-activation-v1`.** User request, 2026-09-13. Lets one client
  politely request that another be raised/focused — e.g. clicking a
  notification should focus the app it's from, or a taskbar's "flash to
  focus" behavior. Without it, the only way to change focus is flexwm's
  own keybindings/IPC; a client has no standard way to ask. Real
  daily-drivability gap, no design work done, presumably a small,
  well-scoped protocol relative to layer-shell/workspace/session-lock.

- **`cursor-shape-v1`.** User request, 2026-09-13. Lets a client (modern
  GTK4/Qt6 toolkits increasingly prefer this) request a named cursor
  shape (e.g. "text", "grab", "not-allowed") without needing to load an
  xcursor theme client-side. Worth noting alongside the still-open
  custom-cursor-theme-name backlog entry above: this protocol is a
  parallel path to that problem, not a duplicate of it — a client using
  `cursor-shape-v1` doesn't need flexwm to have loaded a real xcursor
  theme at all, since the compositor can map the requested shape name to
  its own procedurally-drawn cursor (as items 5/13 already do) rather
  than needing theme assets. Might reduce how much the theme-name gap
  above actually matters in practice, for clients that adopt this
  protocol. No design work done.

- **Smaller/general-client-compatibility protocol gaps, lower urgency,
  bundled here as one entry since none has design work done and none is
  blocking anything else on this list.** User request, 2026-09-13,
  recorded so they don't get lost rather than because any is scheduled:
  - **`wp_presentation`** (presentation-time) — precise frame-timing
    feedback, mainly useful for smooth video/animation clients.
  - **`wp_viewporter`** — lets a client crop/scale its own buffer;some
    clients assume this exists.
  - **`single-pixel-buffer-v1`** — a trivial protocol for a client to get
    a solid-color 1x1 buffer without allocating a real one; some toolkits
    use it for cheap fills.
  - **`relative-pointer-unstable-v1`** — raw, unaccelerated pointer deltas;
    pairs with the `pointer_constraints` support already present, and
    games/3D apps expect both together, not just pointer lock/confinement
    alone.
  - **`fractional-scale-v1`** — crisp non-integer output scaling. Not
    urgent while flexwm has exactly one output and no real scale
    configuration story yet, but relevant once multi-output/HiDPI does.
  - **`text-input-v3`/`input-method-v2`** — IME support for non-Latin
    script input, and on-screen keyboards. A real gap for non-US-keyboard
    daily use; unrelated to item 14's `flexwm msg type`/`msg key` work,
    which is about agent-driven synthetic input, not live IME composition
    from a real input method.

- **Niche protocol gaps, lowest priority for this project's current scope,
  bundled for the same reason as the entry above.** User request,
  2026-09-13:
  - **`tablet-v2`** — drawing-tablet (Wacom-style) input support.
  - **`wlr-screencopy-unstable-v1`/a newer `ext-image-copy-capture-v1`** —
    lets third-party tools (`grim`, `wf-recorder`, screen-sharing in video
    conferencing apps) capture the screen directly, rather than going
    through flexwm's own bespoke `flexwm msg screenshot` IPC action. Worth
    revisiting against `CLAUDE.md`'s "prefer the standard protocol over a
    bespoke one" rule at some point — flexwm's own screenshot action
    exists because computer-use automation needs it under flexwm's own
    control/auth model, but that doesn't mean third-party tools shouldn't
    also have the standard path available to them.
  - **`wlr-output-management-unstable-v1`** (or a newer successor) — lets
    tools like `wlr-randr`/`kanshi` query and reconfigure output mode,
    position and scale. Moot while flexwm has exactly one `Output` and no
    real multi-monitor support; relevant once that lands.
  - **`security-context-v1`** — lets a compositor scope what a sandboxed
    client (e.g. a Flatpak) is allowed to do. Relevant for hardened setups,
    not a natural fit with this project's current minimalist scope.
  - **`content-type-v1`, `alpha-modifier-v1`** — minor rendering hints (a
    client declaring "I'm showing video/a game," or setting whole-surface
    opacity without compositing it itself). Low value on a CPU/pixman-only
    renderer with no adaptive-sync or GPU compositing story.

**From `flexwm-reviewer`'s pass on PR #13 (item 8, client cursor surface
rendering), all low priority, none blocking:**

- **`Cursor::element`'s fallback path allocates a one-element `Vec` every
  rendered frame (LOW).** The old code returned `Option<...>` with no
  allocation; now every `--tty` frame showing the default cursor (the
  common case) allocates and frees a `Vec` to hold it. Not observable in
  benchmarking (12+4 interleaved reps, no measurable difference), and
  `render()` already builds a few per-frame `Vec`s this way, so it matches
  local convention rather than breaking it — but a cheaper shape exists if
  it ever matters: have `element()` append into a caller-owned
  `&mut Vec<CursorElement<R>>` instead of returning a fresh one; a full fix
  (a persistent element buffer on `Backend`) is a larger refactor than fits
  here. The `Surface` path allocates regardless of this fix, since
  Smithay's own `render_elements_from_surface_tree` returns a `Vec`.
- **An animated cursor client keeps getting woken while the `--tty` session
  is VT-paused (LOW, inherited not introduced).** `Tty::present` early-
  returns on `!active`, but `render()` and the frame-callback loops run
  regardless — identical to the pre-existing per-window `send_frame` loop;
  item 8 just makes the cursor share it. Not new, not specific to cursors.
- **`wl_surface.offset` on a cursor surface doesn't move the hotspot (LOW,
  upstream gap).** Per `wayland.xml`, `hotspot_x`/`hotspot_y` should
  decrement on `wl_surface.offset` requests to a cursor surface. At the
  pinned Smithay rev, `CursorImageAttributes.hotspot` is only ever written
  by `wl_pointer.set_cursor` (and the tablet-tool equivalent) — nothing
  adjusts it on offset/commit — and flexwm reads it verbatim. A client
  using `wl_surface.offset` on its cursor gets a misplaced image. Not a
  regression (nothing rendered for `Surface` before item 8), and real
  toolkits don't appear to do this in practice.

**Security audit (2026-09-12, against `main` at `2b92928`).** A dedicated
security pass separate from the usual correctness/performance review found
one CRITICAL finding (any Wayland client can abort the whole compositor via
`wl_shm_pool.resize(0)`, a missing `return` in the pinned Smithay revision —
fixed as item 7 above, not listed here as backlog) and one HIGH finding
(the dev VM's forwarded SSH port bound to every interface instead of
loopback, exposing the documented hardcoded
credentials — and, since the shared `/mnt/flexwm` 9p mount has no read-only
option in this NixOS module, LAN write access to the actual host checkout —
to the whole LAN; fixed same-day, `host.address = "127.0.0.1"` added to
`vm/configuration.nix`, see its own commit and `vm/README.md`). The MEDIUM
and LOW findings below are real but lower-urgency; none were exploitable
data-loss/RCE in what was checked.

- **~~Client-declared `min_size` isn't clamped where it's read (MEDIUM)~~ —
  DONE as item 12(b).** One correction to the diagnosis below, found by
  disabling the clamp: `ring_rects` is not the first unchecked add an
  `i32::MAX` minimum reaches — `flexwm_core`'s own `World::place_workspace`
  (`x + width`) overflows first, inside `arrange`, before anything renders. So
  "this doesn't reach a bad write today" was true of the *write* but not of the
  arithmetic: a debug build panics in the core. Original diagnosis, left as
  written:
  `shell.rs`'s xdg_toplevel handling reads a client's `min_size` straight
  from `SurfaceCachedState` with no clamp, unlike `learned_min` (already
  capped to the output's usable area). Traced the full chain to
  `decorations.rs`'s `ring_rects`, which does unchecked `i32` arithmetic on
  it (`rect.w + 2 * width`) — overflows for a `min_size` near `i32::MAX`,
  though `clip()` currently re-bounds against the real screen size
  regardless, so this doesn't reach a bad write today. The gap is real
  anyway: nothing stops a future consumer of `WindowState::min()` from
  inheriting the unclamped value without `decorations.rs`'s defensive
  re-clip. Fix: clamp at the read site in `shell.rs`, same shape as
  `learned_min`'s existing clamp.
- **~~IPC control socket has no line-length cap (MEDIUM)~~ — DONE as item 9**,
  for the compositor's inbound request path only (`ipc/line.rs`, 1 MiB).
  `flexwm-ipc`'s `read_message`/`read_message_buffered` is deliberately left
  uncapped: the same codec reads `Response::Screenshot`, which is legitimately
  several MB of base64 PNG, so a cap there would break real screenshots. A
  future `Client`-side cap would have to be per-message-type, not global.
- **A `[binds]` entry naming a capital letter parses, loads, and can never
  fire (LOW, pre-existing).** `"A" = "close"` is accepted without a
  warning, but `input.rs`'s `keysym_named` tries the name *exactly* first,
  which for a single letter always succeeds and yields the distinct `A`
  keysym — while `keybindings::match_key` is fed `handle.raw_syms().first()`,
  the key's unshifted symbol, which is `a`. The two never meet. Found while
  correcting this branch's documentation (the README claimed letters
  "always resolve to their lowercase keysym", which is only true when the
  exact lookup *fails*); reproduced on `--headless` in
  `/home/dev/bindcase.sh` on the dev VM: with `"A" = "close"` the window
  survives `flexwm msg key shift+a`, with `"shift+a" = "close"` it closes.
  The README now says to write `"shift+a"`. Fixing it properly means
  lowercasing a single-letter key name at *config parse* time only —
  deliberately not in `keysym_named` itself, since `flexwm msg key A` must
  keep refusing rather than silently becoming `a` (see the resolved entry
  above) — plus a test per direction. Worth doing next time `config.rs` is
  open; nothing silently misbehaves in the meantime, the bind simply does
  nothing.
- **A large `flexwm msg type` blocks the whole event loop for its whole
  duration (LOW, pre-existing).** `ipc.rs` runs `Request::Type` to
  completion synchronously on the sole event-loop thread, so a request at
  the 1 MiB inbound cap (item 9's `ipc/line.rs` limit, which is what bounds
  how big this can get) stalls wayland dispatch, input and rendering until
  every character has been sent. Pre-existing — `type_text` has always been
  synchronous, and `git blame` puts it well before the shifted-character
  work — but PR #23's own benchmark numbers put a figure on it and roughly
  doubled the worst case: ~2.3 s per MiB before, ~2.4 s if the text is all
  lowercase, ~3.9 s if it is all shifted (extrapolated from the measured
  medians of 112 ms and 188 ms per 50,000 characters, release build,
  `--headless`), because a shifted character correctly costs four key
  events instead of two. Nothing an agent does deliberately gets near this
  — a shell command line is a few hundred characters, i.e. under a
  millisecond — so this is a hostile-input/accident bound, not an everyday
  cost. Fixing it properly means chunking the request across event-loop
  iterations (send N characters, yield, resume), which needs per-connection
  progress state of the kind item 10 deliberately avoided; a much cheaper
  partial answer is a separate, smaller cap on `Request::Type`'s text
  specifically, refused rather than delayed, the same shape the screenshot
  rate limit took.
- **Screenshot capture runs synchronously on the sole event-loop thread with
  no rate limit (MEDIUM)** — **the rate limit is DONE as item 9** (one
  capture per connection per 16ms frame, refused rather than delayed). Still
  open, and deliberately out of scope there: the IPC accept loop has no cap
  on concurrent connections, so the per-connection limit is bypassable by
  reconnecting for every capture, and nothing bounds how many connections one
  client can hold open. Also still true, and unaffected by rate limiting: the
  capture itself is synchronous on the event-loop thread, so each one stalls
  wayland dispatch and input for its duration (~12ms at 1600x1000 in a
  release build, measured in item 9). Moving the encode off-thread is a much
  larger change than the limit was. **One more lifecycle case belongs here,
  found by `flexwm-reviewer` while reviewing item 10**: a client that
  half-closes (`shutdown(SHUT_WR)`) and then never reads pins its connection
  slot and two fds for good. A half-close raises `EPOLLIN`/`EPOLLRDHUP`, not
  `EPOLLHUP`, and a connection with a queue is registered for writability
  only, so nothing wakes it again. Deliberately not fixed in item 10:
  registering for reads there would spin the loop at full speed on an
  end-of-stream that can never be acted on (the queue cannot drain), which is
  worse. Strictly better than the pre-item-10 behavior, where that same case
  froze the whole compositor — but still a live resource leak a connection
  cap would need to account for.
- **~~A real keystroke (libinput, or nested host-forwarded input) can sit
  unflushed to the client for seconds on a quiet screen (MEDIUM)~~ — DONE as
  item 11**, PR #17, fixed exactly as suggested here (the one-line
  post-dispatch flush) and re-verified on real `--tty` *and* `--nested`
  hardware both before and after. Two notes on this entry's own framing: the
  "matching Smithay's own `anvil` pattern" half is true in substance but not in
  form -- anvil hand-rolls its dispatch loop rather than using
  `EventLoop::run`'s callback (see item 11) -- and the closing suggestion that
  item 10's `ipc/connection.rs` invariant "would likely be simplified too" was
  deliberately *not* acted on: it is well-tested, three review rounds old, and
  redundant-but-earlier rather than wasteful, since a flush with nothing queued
  issues no syscall. Original diagnosis, left as written: found by
  `flexwm-reviewer` while reviewing item 10, unrelated to and not caused by
  that PR (confirmed identical on `main` before it). Reproduced twice on
  real `--tty` with a genuine `/dev/uinput`-injected key: after the press,
  total IPC silence for 6s, then the very next screenshot's own end-of-
  wakeup flush is what finally delivers the character to the client — one
  screenshot shows the key hasn't visibly arrived yet (pixel-identical to
  before the press), the next one 1.5s later shows it has, with no new input
  or IPC traffic in between other than the screenshots themselves. Chain:
  `tty/mod.rs`'s `libinput_event` queues the client's `wl_keyboard` message,
  but nothing in the keyboard path calls `request_render()` (only pointer
  motion does, and only under `--tty`) or flushes on its own; `render()`
  early-returns before its own flush when `!needs_render`; the frame timer
  drops itself when idle. So on an otherwise-quiet screen, a keystroke is
  invisible until something else (mouse motion, another IPC request)
  happens to trigger a flush. Item 10's fix closed this exact shape for the
  IPC-injected-input path specifically; this is the same root cause on the
  real-hardware-input path, which item 10 doesn't touch. The idiomatic fix:
  a single `let _ = state.display_handle.flush_clients();` in the
  post-dispatch callback of `event_loop.run` (`compositor/mod.rs`, currently
  a no-op `|_| {}`) — matching Smithay's own `anvil` reference compositor's
  pattern — would make "every dispatched event eventually gets flushed"
  structural instead of a per-call-site discipline to remember, and would
  likely let item 10's own hand-maintained "every exit reaches the flush"
  invariant in `ipc/connection.rs` be simplified too.
- **~~IPC socket has no explicit permissions or peer-credential check
  (LOW/MEDIUM)~~ — DONE as item 9.** Both halves: `0600` on the socket file
  and a same-uid `SO_PEERCRED` check at accept time, proven independent of
  each other on hardware. One correction to this entry's diagnosis, found
  while fixing it: under the ordinary `umask 022` the socket came out
  `srwxr-xr-x`, which other users cannot connect to anyway (connecting needs
  write permission) — so "silently degrades to any-local-user access" was
  true for a lax umask (demonstrated with `umask 000`), not for a
  `FLEXWM_SOCKET` override alone.
- **`--tty`'s explicit `O_CLOEXEC` request on the DRM fd is a no-op at the
  libseat layer (LOW, informational).** `tty/mod.rs` requests
  `OFlags::CLOEXEC` when opening the DRM device, but the pinned Smithay's
  `LibSeatSession::open` discards the flags parameter entirely and just
  calls `libseat::Seat::open_device` — so whether an `Action::Spawn`-launched
  child inherits DRM master or input-device fds depends entirely on
  libseat's own C-side behavior, not on flexwm's request. **Measured
  2026-09-13** with exactly the check this entry suggested, while an
  SSH-started `--tty` ran on the dev VM at `0678765`: every one of flexwm's
  seatd-obtained fds (`/dev/dri/card0` and all four `/dev/input/event*`)
  reports `flags: 02504002`, which has the `02000000` `O_CLOEXEC` bit set. So
  the outcome flexwm asked for does hold (an `Action::Spawn`ed child inherits
  neither DRM master nor input fds), just for a different reason than
  flexwm's own argument. Correction, caught by `flexwm-reviewer`: close-on-
  exec is a property of the *receiving process's own fd table*, not of the
  open file, and `SCM_RIGHTS` (how the fd crosses the seatd-flexwm socket)
  never transfers it, so "identical to seatd's fd, since it's the same open
  file" is the wrong reason for this one bit -- the rest of `02504002`
  genuinely is shared `f_flags` from that open file
  (`O_RDWR|O_NONBLOCK|O_NOFOLLOW|O_LARGEFILE`), just not this one. The actual
  guarantor is libseat's own receive call:
  `recvmsg(..., MSG_DONTWAIT | MSG_CMSG_CLOEXEC)`
  (`libseat/common/connection.c:202`) sets close-on-exec the moment libseat
  receives the fd into flexwm's own process, unconditionally, for every fd it
  hands over. A real, deliberate contract -- just libseat's, not flexwm's
  `OFlags::CLOEXEC` argument, which remains the no-op this entry originally
  found. Still informational, still no code change: if a future libseat
  version ever dropped `MSG_CMSG_CLOEXEC`, every `Action::Spawn`ed child
  would silently inherit DRM master and every input device fd, and nothing
  in flexwm's own code would catch or even notice it.
- **~~IPC socket path is unlink-then-bind (LOW, non-default config only)~~ —
  DONE as item 9.** The original framing here was wrong (`bind(2)` does not
  follow a trailing symlink, verified — `EADDRINUSE`), so the real exposures
  were the chmod-after-publish window and losing the name to a racing
  process. The fix that shipped is not a simple bind-to-temp-name-then-
  rename, though: two review rounds found that shape itself introduced two
  more symlink/length bugs of its own (see item 9's own write-up for the
  full history). The final design claims an unpredictable
  (`getrandom`-sourced), short staging name via `mkdir` itself — never an
  unlink or a name a pre-planted symlink could sit at — then does every
  subsequent operation (chmod, bind, the rename's source side) through an
  `O_DIRECTORY | O_NOFOLLOW` fd via `/proc/self/fd/<n>/...`, never
  re-resolving the staging path by name again. Only the final rename's
  *destination* (the published path itself) is still name-resolved, which
  is inherent to what a published path is.

- **~~The IPC connection loop does blocking I/O, one line per readiness event
  (HIGH; pre-existing)~~ — DONE as item 10.** The diagnosis below is left
  as-written because it is what the fix was built against; item 10 records what
  shipped, which part of this diagnosis turned out slightly off, and what it
  deliberately left alone. Found while bug-bashing item 9;
  every symptom below verified identical on the pre-item-9 binary, so none is
  a regression, and all were explicitly left unfixed there. Raised from MEDIUM
  to HIGH by `flexwm-reviewer`'s pass on PR #14, which reproduced symptom (a)
  more precisely than item 9's own bug bash did: a half-written request line
  does not merely hang that one connection, it freezes the *entire* event loop
  — `/proc/<pid>/wchan` reads `unix_stream_read_generic` and CPU time goes
  completely flat, so no frame tick, no wayland dispatch and no input
  processing happens for any client, for as long as one connection holds a
  partial line. It needs no malice at all: a client killed mid-paste, or any
  agent that writes a request in chunks, does it. That makes it the first
  thing to pick up from this backlog rather than one more unordered entry.

  `accept()` puts the connection into *blocking* mode and `Connection::step`
  reads exactly one line per readiness event, which causes three distinct
  symptoms with one root cause:
  (a) a client that writes `{"type":"vers` and holds the connection open
  parks the single event-loop thread inside `fill_buf` — every other IPC
  client, wayland dispatch and input stop until it sends a newline or
  disconnects (confirmed: a second client's perfectly valid request went
  unanswered for a 20s read timeout, then was served in 86us the moment the
  stalled client went away). This is a local hang DoS that needs no
  malice — a crashed agent mid-write does it — and it is a *worse* version of
  the finding item 9's line cap closed, since it costs the attacker one byte
  instead of a megabyte.
  (b) two requests in one `write()` get one reply: the `BufReader` drains
  both from the socket, the level-triggered source sees nothing more to read,
  and the second line sits in the buffer unanswered until unrelated traffic
  wakes the connection. Nothing in-tree pipelines (both `flexwm msg` and
  `flexwm_ipc::Client` are strict request/response), so this is latent, but it
  is a protocol surprise for any agent that batches.
  (c) `reply()` writes blocking too, so a client that keeps sending requests
  and never reads the answers fills its own receive buffer and deadlocks the
  compositor inside `write_all` (confirmed: the probe's own `write_all` blocked
  in turn, before it could even open a second connection to test with, and the
  compositor was answering again 61us after that client was killed). Same
  one-byte-of-effort shape as (a), from the other direction.
  Fix direction, one change for all three: leave the stream non-blocking, have
  the bounded reader return "incomplete, keep what you have" on `WouldBlock`
  (clearing the buffer only once a line has been consumed, so the 1 MiB cap
  still applies across however many reads a line takes), loop `step` over
  every complete line already buffered before returning to the event loop, and
  give each connection an outbound buffer plus `Interest::WRITE` when a reply
  cannot go out in one go — with a cap on that buffer, since it is the same
  unbounded-growth shape item 9 just closed on the read side. That restructures
  the connection loop and wants its own test matrix (partial lines interleaved
  with `WaitIdle`'s hand-off and the screenshot limiter), which is why it is
  its own item rather than a rider on item 9.
- **`--width`/`--height` are unbounded `i32`s (LOW, operator-supplied).**
  Found while bug-bashing item 12 and deliberately left out of it. Item 12(b)
  bounds a *client's* `min_size` to the output's usable area, and 12(c) bounds
  the gap, but the output's own size still comes straight from
  `cli.rs`'s `number("--width", ..)` with no range check — so
  `--width 2000000000` can overflow the same `x + width` in `arrange.rs`'s
  on-screen test that 12(b) closes for minimums, plus `Rect::right()`/
  `bottom()` wherever those are read. Lower priority than the four in item 12
  because it needs the operator to pass an absurd flag to their own
  compositor rather than a client or a config file to declare one, but it is
  the same family of fix (clamp at the read site, with a documented bound)
  and would close the last unbounded input to the layout's arithmetic. A
  realistic bound is whatever DRM itself can report for a mode, with room to
  spare. Two more facts `flexwm-reviewer` found while reviewing item 12,
  worth fixing alongside this rather than separately: an absurd `--width`
  doesn't just overflow directly — since 12(b)'s `min_size` limit is
  *derived from* the output's usable area, a huge enough output makes that
  clamp effectively vacuous (the limit becomes ~2×10⁹, which bounds nothing
  real); and `Rect::inset`'s `self.x + by`/`self.y + by` are unguarded even
  though its `w`/`h` arms already floor at 0 — the same function, only half
  hardened.
- **~~Config parsing has no recursion-depth guard (LOW)~~ — RESOLVED
  2026-09-13 (investigation + regression tests, PR #21): at `toml` 1.1.6 the
  premise doesn't hold at the default stack rlimit (8 MiB, unmodified
  `ulimit -s`) — the crate guards nesting itself, at 80 levels, and anything
  deeper lands in exactly the "log and fall back to defaults" path the entry
  worried it would bypass. No production code change; a guard of flexwm's own
  would be a second, worse bound on top of a precise one. The guard bounds the
  depth, though; it does not make the parse free, so the conclusion is scoped
  to that 8 MiB budget — a *debug* build launched under `ulimit -s 2048` does
  still abort on the worst case (measured, below).**
  Original diagnosis, left as written: the `toml` stack has no explicit guard
  against deeply nested input; a maliciously deep config could
  stack-overflow-abort the process rather than hit the module's normal "log
  and fall back to defaults" path. Requires the user's own config file, so low
  priority.

  **What the crate actually does.** `toml` 1.1.6 (over `toml_parser` 1.1.3 and
  `winnow` 1.0.4) bounds depth in two independent places, both hard-coded at
  80: a `RecursionGuard` around combined inline-table/array nesting
  (`toml/src/de/parser/mod.rs:37,58,71`), which the parser consults — it
  returns `false` past the limit and `on_array_open`/`on_inline_table_open`
  switch to the iterative `ignore_to_value_close` skip instead of recursing
  (`toml_parser/src/parser/document.rs:796,962,1480`) — and a separate cap on
  dotted-key/table-header path segments (`toml/src/de/parser/key.rs:66`). Both
  are active unless the crate's `unbounded` feature is on; it is not, but the
  command matters: `unbounded = []` enables nothing else, so `cargo tree
  -e features` prints the identical `default, display, parse, serde, std`
  whether it's on or off and can't be used to check this. `cargo tree -p
  flexwm --target all -f "{p} | {f}"` is the one that actually shows it —
  `toml v1.1.6+spec-1.1.0 | default,display,parse,serde,std`, no `unbounded`
  suffix — confirmed by building a probe with the feature on and watching the
  suffix appear.

  **Evidence.** Scratch harness (kept out of the tree; the cases that matter
  are now tests in `crates/flexwm/src/compositor/config.rs`), aarch64 on both
  macOS 26 and the Linux dev VM, `ulimit -s` 8192 KiB on the Linux dev VM and
  8176 KiB on macOS (the actual default there, not 8192 -- doesn't move any
  number below, since every macOS figure was measured on an explicitly-sized
  spawned thread rather than against the main-thread rlimit):
  - *Boundary*, through flexwm's real `toml::from_str::<FileConfig>` path: 80
    levels parse and are then rejected by `deny_unknown_fields` ("unknown
    field a"); 81 are refused by the crate (`cannot recurse further; max
    recursion depth met` for inline tables and arrays, `recursion limit` for
    dotted keys, `[a.a…]` headers and `[[a.a…]]` headers). Identical on both
    platforms. A 1,000,000-level file (4 MB) returns the same clean error in
    milliseconds, no crash.
  - *Counterfactual*, the same harness built with `toml/unbounded`, Linux, main
    thread, `ulimit -s 8192`, one process per cell over the depth ladder
    {1k, 2k, 5k, 10k, 20k, 50k, 100k, 200k}. Every nesting form does eventually
    abort with `fatal runtime error: stack overflow` (SIGABRT, exit 134), which
    is the point — the crate's guard is load-bearing, not incidental — but
    *where* it aborts varies by an order of magnitude between forms and by
    5–10× between profiles, so there is no single "N levels is fine" number:

    | nesting form           | release: ok / first abort | debug: ok / first abort |
    | ---------------------- | ------------------------- | ----------------------- |
    | inline table           | 5,000 / 10,000            | 1,000 / 2,000           |
    | array                  | 10,000 / 20,000           | 2,000 / 5,000           |
    | dotted key             | 50,000 / 100,000          | 5,000 / 10,000          |
    | table header           | 50,000 / 100,000          | 5,000 / 10,000          |
    | array-of-tables header | 50,000 / 100,000          | 5,000 / 10,000          |

    (The spread is consistent with inline tables and arrays costing a parser
    frame per level *on top of* the drop frame every form pays, which is also
    why those two are the forms `RecursionGuard` watches — but that reading was
    not traced through `document.rs`'s key-path insertion, only inferred from
    these numbers.)
  - *Composition ceiling*, the one genuinely surprising result: the two limits
    **multiply** rather than add, because a dotted key can sit at every level
    of nesting. The deepest file is three parts, each at its own cap: an
    80-segment **array-of-tables** header (81 levels — its last segment is an
    array holding a table, one more than a plain `[a.a…]` header spends), then
    80 nested inline tables each keyed by a fresh 80-segment dotted key (80
    levels apiece), then an 80-segment dotted key for the leaf (79 more).
    13,448 bytes, 6,561 nested tables/arrays counting the document root (the
    scalar at the bottom is the 6,562nd node on that path) — and nothing a
    config file can express goes deeper. Two shallower spellings measured for
    contrast, both of which parse all the way through: a plain `[a.a…]` header
    with the same body is 6,560 (13,446 bytes), and with a plain `a = 1` leaf
    instead of a dotted one it is 6,481 (13,288 bytes).
  - *Where the stack goes*: not the parse — the recursive **drop** of the
    parsed `DeTable`. Minimum surviving thread stack for that 13,448-byte worst
    case at flexwm's `FileConfig` target, Linux, binary-searched in 4 KiB (one
    page) steps with one process per trial:

    | | release | debug |
    | --- | --- | --- |
    | parse only (`DeTable::parse` + `mem::forget`) | < 134 KiB | 476 KiB |
    | parse + drop (`toml::from_str::<FileConfig>`) | 932 KiB | 6,680 KiB |
    | the committed test's whole body (`load_from`) | 1,140 KiB¹ | 6,684 KiB |
    | a `toml::Table` target instead | 7,288 KiB² | 32,100 KiB |

    ¹ `cargo` ignores `profile.release.panic` for test targets, so a release
    *test* binary unwinds where the shipped binary aborts, and on the two
    `FileConfig` rows its landing pads cost ~200 KiB more: the same parse+drop
    measures 932 KiB with `panic = "abort"` (what flexwm ships) and
    1,136–1,140 KiB with unwind (what `cargo test --release` builds). The old
    1,135 KiB figure here was an unwind measurement; both are recorded now so a
    re-measurement matches whichever was run.

    ² The `toml::Table` row moves the *other* way between panic strategies:
    7,288 KiB with `panic = "abort"`, 6,468 KiB with unwind (measured both in
    the scratch probe and in flexwm's own release test binary). 7,288 is the
    headline because it is both the shipped profile and the conservative
    number, but a reviewer re-measuring via `cargo test --release` should
    expect 6,468, not something above 7,288.

    Release "parse only" has no point value on Linux via a thread-stack sweep:
    glibc floors a thread stack at 137,120 bytes here and the parse survives
    that floor, so all that can be said is "< 134 KiB". macOS *is* measurable
    there (84 KiB) — the old "79 KiB" attributed to Linux was a macOS number.
    Of the five rows with a Linux point value to compare against, macOS tracks
    Linux 16–336 KiB lower (0.3–5%) on all five, never higher — as parse+drop
    / `toml::Table` / parse-only: release 916 / 7,268 / 84 KiB, debug 6,660 /
    31,764 / 452 KiB.

    flexwm parses on the main thread (`main` → `compositor::run` →
    `config::load`), so it has 8 MiB: over 7 MiB spare in release, 8,192 −
    6,684 = 1,508 KiB (~1.5 MiB) spare in debug. The `toml::Table` row is the
    one that nearly runs out — it fits in release with 904 KiB to spare, and
    does not fit at all in debug.
  - *End to end* at `3cb51fc`, binaries rebuilt from that tree (this PR moved
    only doc comments and the test's worst-case bytes, so the production code
    here is unchanged from `e6a983a`'s — but the file under test is not, hence
    the re-run), with the real debug binary on the dev VM
    (`/var/cargo-target/debug/flexwm --headless --config …`): all three 13 KB
    worst-case files (under an unknown key, under `[binds]`, under `[layout]`;
    13,448 / 13,452 / 13,453 bytes) plus the 4 MB 1,000,000-level file started
    normally, logged `ERROR … could not parse config file; using defaults`, had
    no `stack overflow` line, answered `msg version` over IPC, and shut down
    cleanly on `SIGTERM` (exit 143). A legitimate config alongside them loaded
    normally.
  - *Reduced stack rlimit*, same `3cb51fc` binaries and the 13,448-byte file: a
    debug build under `ulimit -s 2048` prints `thread 'main' has overflowed its
    stack` / `fatal runtime error: stack overflow, aborting` and dies with
    SIGABRT (exit 134) — the 6,684 KiB it needs does not fit in 2 MiB. The
    release build is fine there (932 KiB), and both are fine at the default
    8192. This is why the resolution above is scoped to the default rlimit.
  - *The committed test's own margin*, measured inside flexwm's debug test
    binary by temporarily making its thread size settable: aborts at 6 MiB,
    passes at 7 MiB — consistent with the 6,684 KiB above, and why the test
    runs on an 8 MiB thread (what production gets) rather than `cargo test`'s
    2 MiB.

  **Recorded rather than fixed**, in `parse_or_defaults`' doc, because it is
  invisible at the site that would break it: moving config parsing to a
  spawned thread would get the Rust default 2 MiB and abort a debug build on
  that 13 KB file; launching under a reduced `ulimit -s` does the same; and
  deserializing the same bytes into a `toml::Table` (a passthrough config
  section, say) descends the whole tree instead of stopping at the first
  unknown key — 7,288 KiB release, which still fits on the main thread but
  leaves under 1 MiB instead of over 7, and 32,100 KiB debug, which does not
  fit at all.

  **Caveat:** every measurement above is aarch64 (macOS + the dev VM); nothing
  was run on x86_64, and this repo has no CI, so the debug-build margin is
  only exercised where someone runs `cargo test`.
- **~~`flexwm-core`'s `gap` config value has no upper bound (LOW)~~ — DONE as
  item 12(c)**, `Config::MAX_GAP` = 10,000 with `clamp_gap` shared between the
  core's `validated()` and the compositor's focus-ring sizing. Original
  diagnosis, left as written: clamped
  only at the bottom (`.max(0)`); a very large configured gap can overflow
  plain `i32` arithmetic in `layout.rs`/`arrange.rs`. Config-only, same fix
  shape as the `min_size` finding above.
- **~~No upper bound on *per-pool* shm size (LOW)~~ — DONE as item 12(d)**,
  512 MiB at both `create_pool` and `resize`, refused rather than clamped.
  The entry's "a cap on pool size at creation/resize time would close it" is
  exactly what shipped for one pool; what it did not anticipate is that
  refusing `create_pool` leaves an uninitialized object whose `request` is a
  `panic!` (safe, because `post_error` kills the client synchronously — see
  item 12(d) for the full argument, including a second, independent panic
  site `flexwm-reviewer` found and confirmed is covered by the same fact).
  **This closes the per-pool case only — the *total* across many pools from
  one client is still unbounded, see the new entry directly below**, found
  by `flexwm-reviewer` while confirming this one.
  Original diagnosis, left as written: found while verifying item 7's
  fix, not by the original audit. `wl_shm_pool.resize(i32::MAX)` (or
  `wl_shm.create_pool(fd, i32::MAX)` directly — reaches the same `mmap`,
  not specific to `resize`) is accepted, reserving a ~2 GiB mapping per
  pool, repeatable per pool and per connection, with no cap anywhere.
  Verified live: no error, no crash — Smithay's own SIGBUS handler covers
  reads/writes past the backing fd's real size, so this isn't the same
  memory-safety class as item 7's bug, just unbounded address-space/fd
  reservation. Same family as the IPC line-length and screenshot-throttling
  findings above (resource exhaustion, not memory corruption) — a cap on
  pool size at creation/resize time would close it.

- **No upper bound on *total* shm reservation per client (LOW/MEDIUM).**
  Found by `flexwm-reviewer` while confirming item 12(d)'s per-pool cap
  actually closed the original finding — it doesn't, fully. `dispatch.rs`'s
  `MAX_SHM_POOL_BYTES` (512 MiB) bounds one pool, but nothing bounds how many
  pools one client opens: 40 pools each at exactly the cap reserve ~20 GiB
  from a single connection, more address space than the pre-fix 8-pool/16.1
  GiB finding item 12(d) was written to close, just needing more requests to
  get there. Not the separate IPC-connection-cap Backlog entry's territory
  (that one is about the control socket's own connection count, unrelated to
  wayland client accounting). Fix direction: per-client cumulative tracking
  in the same `dispatch.rs` interception point, with its own cap — needs
  deciding what identifies "one client" for accounting purposes (the
  `ClientId` `dispatch.rs` already has access to) and where to hang the
  running total (`ClientState`, most likely, alongside the existing
  per-client data already tracked there).

- **Suppress the same-VT no-op case of the IPC VT-switch warning (5c).**
  `change_vt`'s `VtSwitchOutcome::Requested` also fires — with a hedged
  warning, per 5c — when the requested VT is the one the session is already
  showing on, since libseat itself returns `Ok(())` for that request too
  (confirmed on real hardware; see 5c's verification). A precise fix would
  need `Tty` to track which VT it currently occupies and compare before
  calling `session.change_vt`, so this case can be `Ignored` instead of a
  hedged `Requested`. Not done in 5c because libseat exposes no query for
  "what VT is this session on" at `init` time — the number would have to be
  sourced some other way (the kernel's own active-VT ioctl on the console
  fd, perhaps) and 5c's warning-not-guarantee wording already makes this a
  false-positive risk, not a silent-failure one, so it wasn't worth blocking
  5c on. Low priority: cosmetic (an agent gets told to be careful once for
  no reason), not correctness-affecting.

- **Custom/client cursor support — (a) DONE as item 8; (b) DONE for size and
  color as item 13; a theme *name* remains open and needs a real licensed
  asset source first.**
  Item 5 shipped a fixed, procedurally-generated triangle for every
  `CursorImageStatus` variant — `Named` (a requested xcursor theme name) and
  `Surface` (a client-supplied cursor image, e.g. a text-input I-beam or a
  resize arrow) both drew the exact same shape, ignoring what was actually
  requested. ~~(a) honor `CursorImageStatus::Surface` by rendering the
  client's actual supplied buffer as the cursor element~~ — landed as item 8
  above. ~~(b) a user/config-level override for the fallback shape's size and
  color (`[appearance]`'s `cursor_size`/`cursor_color`)~~ — landed as item 13
  above, which is what `Named` will always fall back to: there is no client
  buffer behind a `Named` request, only a theme name.

  **Still open, and deliberately scoped out of item 13: drawing a different
  shape per requested theme name.** That needs either an actual cursor-theme
  asset this project is allowed to ship (niri's are GPL, Adwaita's aren't
  MIT-clean per `CLAUDE.md`, and nothing MIT-clean has been found or vetted
  yet) or real xcursor-file loading infrastructure — a much larger feature
  than a config knob, and one that is pointless without an asset to load. So
  this stays blocked on sourcing a license-clean theme, not on code. Until
  then `Named` draws item 13's configurable triangle whatever shape was
  asked for, which is at least a visible, user-tunable pointer rather than a
  wrong-shaped fixed one.

- **Rename the project from `flexwm` to `flex`, as part of a small family of
  tools: `flex` (the compositor), `flexctl` (a CLI/IPC client), `flexbar` (a
  companion status bar).** User request, 2026-09-13 — not scoped or
  scheduled yet, recorded here so it doesn't get lost. This is bigger than a
  find-and-replace, in two separable ways:

  **1. The rename itself.**

  - **Crates.** All three workspace members are named for it:
    `crates/flexwm` (binary, package `flexwm`), `crates/flexwm-core`
    (platform-independent state/layout), `crates/flexwm-ipc` (the IPC
    protocol crate). Renaming the packages means every internal
    `flexwm-core = { path = ... }`/`flexwm-ipc = { path = ... }` dependency
    line, every `use flexwm_core::...`/`use flexwm_ipc::...` import, and the
    binary target name (`cargo build -p flexwm` → whatever the new package
    is called) all move together — a mechanical but wide-reaching change,
    not a one-line edit.
  - **The GitHub repo** is `yackey-labs/flexwm`. A GitHub rename leaves a
    redirect from the old URL, but every local clone's `origin` remote
    still points at the old name until updated by hand (`git remote
    set-url`), and anything that hardcodes the URL (this repo's own
    `Cargo-Session`/attribution lines in past commits, any external bookmark
    or CI config) won't follow the redirect automatically. Coordinate the
    repo rename with updating local remotes in the same sitting, not as an
    afterthought.
  - **Everything textual**: `README.md`, `ROADMAP.md` itself (including
    every historical entry that names `flexwm` — decide whether history
    gets rewritten or just new entries use the new name), `vm/README.md`,
    `vm/configuration.nix` (service/user names, paths), `HANDOFF.md`,
    `scripts/smoke-test.sh` and any other script that invokes the binary by
    name, and the `flexwm msg`/`flexwm --tty`/etc. CLI surface itself (which
    is also user-facing documentation, per this project's "compositor, not
    window manager"-style naming rules in `CLAUDE.md`).
  - **Open question, not yet decided**: do the `.claude/agents/*.md` role
    files (`flexwm-implementer.md`, `flexwm-reviewer.md`,
    `flexwm-orchestrator.md`) and the subagent names they're invoked under
    rename too, for consistency? They're project tooling rather than
    product surface, so this could reasonably go either way — flag it for a
    decision when this is actually scoped, don't assume either answer here.
  - **Worth a quick check before committing to `flex`**: whether that name
    collides with anything relevant (an existing crates.io crate, if this
    is ever meant to be published; an existing well-known `flex` CLI tool
    a user might have on `$PATH`, e.g. GNU flex the lexer generator, which
    is a real, extremely common collision to be aware of before locking in
    a bare `flex` binary name).

  **2. `flexctl` implies splitting the CLI out of the compositor binary,
  not just renaming it.** Today `flexwm msg ...` (and `type`/`key`/etc.) is
  one `Command` variant of the single `flexwm` binary (`crates/flexwm/src/
  cli.rs`) — the same executable that starts the compositor also sends it
  IPC requests, distinguished by argv. A separate `flexctl` binary is a real
  design decision, not a rename: does the compositor crate stop exporting a
  CLI at all and become `flex --tty`/`flex --nested`/`flex --headless`
  only, with everything under `msg` moving to a new crate/binary that talks
  the same Unix-socket protocol from the outside? That would cleanly
  separate "the compositor" from "a client of the compositor" (useful for
  the computer-use goal specifically — an agent shells out to `flexctl`,
  not to the compositor's own binary) but is a real crate-boundary change,
  probably wanting its own design pass rather than riding along with a
  find-and-replace rename.

  **3. `flexbar` would be a new project**, not a rename of anything that
  exists: a status bar built as a `wlr-layer-shell-unstable-v1` client
  (item 14, PR #22), presumably filling the same niche as `waybar` but
  purpose-built for this compositor. Worth doing at some point — flexwm/flex
  has no bar of its own today, and every hardware screenshot/demo of the
  layer-shell work so far uses `waybar`, a third-party dependency, to prove
  the protocol works — but it is a whole new binary/crate with its own
  scope, feature set and release cycle, not a line item inside the rename.
  Should probably be scoped as its own separate roadmap item once the
  rename (and the `flexctl` split, if that's the direction) land, rather
  than being designed as a rider on this entry.

- **`flexwm msg key`'s modifier resolution hard-codes `Shift_L`/`Control_L`/
  `Alt_L`/`Super_L` and requires each at level 0, so a layout that moves a
  real modifier off its `_L` key breaks `msg key` combos entirely (LOW,
  pre-existing).** Found by `flexwm-reviewer` while re-verifying item 14's
  shifted-character fix (that PR's own `ModifierKeys::probe` — which asks
  the keymap which key *actually* holds a given real modifier on the active
  layout/group, rather than assuming a fixed keysym — sits one function away
  from this bug and already knows how to answer it correctly). `input.rs`'s
  `resolve_combo` maps `Modifier::{Ctrl,Shift,Alt,Super}` to a hard-coded
  `_L` keysym and then demands it be reachable at level 0.

  Concrete failure: `XKB_DEFAULT_LAYOUT=us,de XKB_DEFAULT_OPTIONS=grp:lshift_toggle`
  (or `grp:lctrl_toggle`) is a real xkeyboard-config option that removes
  `Shift_L`/`Control_L` from the keymap entirely, leaving `Shift_R`/
  `Control_R` as the only key carrying that real modifier. `flexwm msg type
  "A"` still works (the probe finds `Shift_R`), but `flexwm msg key
  shift+a` and `flexwm msg key ctrl+c` are refused with ``no key for `shift`
  in this layout`` — an agent on such a session cannot send Ctrl+C, or any
  other modifier combo, at all. Verified pre-existing against the release
  binary from before item 14's fix landed, so this isn't a regression from
  that work — but it's the identical bug class (assuming a fixed keysym
  instead of asking the keymap) in the modifier position rather than the
  character position, and the fix is now a short reach: have `resolve_combo`
  go through `modifiers::ModifierKeys` (or equivalent) the same way
  `type_text` already does, instead of a hard-coded keysym table.
