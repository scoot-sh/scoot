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
   shape; `::Surface` was fixed in item 8, `::Named` is still Backlog (b).

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
   pass, clippy/fmt clean, cross-platform build clean on macOS.

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
   config-level override for the *fallback* shape's theme/size/color) is
   still open and deliberately untouched here. Like item 7, this landed
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

## Backlog (unordered — pick up whenever it fits)

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

- **Client-declared `min_size` isn't clamped where it's read (MEDIUM).**
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
- **IPC control socket has no line-length cap (MEDIUM).** `flexwm-ipc`'s
  `read_message`/`read_message_buffered` and `ipc.rs`'s connection loop both
  `read_line` into an unbounded `String`. A connected client streaming bytes
  with no `\n` grows that buffer without bound — single-connection memory
  exhaustion, no special access needed beyond opening the socket. Fix: wrap
  the reader in a bounded `Read::take(N)` (or equivalent) and close/error
  past a sane max line length.
- **Screenshot capture runs synchronously on the sole event-loop thread with
  no rate limit (MEDIUM).** flexwm is single-threaded throughout; a full
  render + framebuffer copy + PNG encode blocks Wayland dispatch, input
  processing, and every other IPC connection for its duration. A client
  hammering `Screenshot` requests is a real "snappy, always" violation this
  project explicitly cares about (`PointerMove`/`Key` at libinput rate is
  fine by design — screenshot is the sharp edge). The IPC accept loop also
  has no cap on concurrent connections, compounding with the line-length
  finding above. Fix direction: rate-limit or de-bounce screenshot requests
  per connection, and/or cap concurrent IPC connections.
- **IPC socket has no explicit permissions or peer-credential check
  (LOW/MEDIUM).** Safety today rests entirely on `$XDG_RUNTIME_DIR` being
  the systemd-default `0700` per-user directory — no `chmod` or
  `SO_PEERCRED` check exists in `flexwm-ipc`/the compositor itself, so an
  explicit `FLEXWM_SOCKET` override into a shared directory, or a non-default
  umask, silently degrades to any-local-user full input-injection +
  screenshot access. Fix direction: explicit `chmod 0600` after bind, and/or
  a peer-credential uid check given this channel's privilege level.
- **`--tty`'s explicit `O_CLOEXEC` request on the DRM fd is a no-op at the
  libseat layer (LOW, informational).** `tty/mod.rs` requests
  `OFlags::CLOEXEC` when opening the DRM device, but the pinned Smithay's
  `LibSeatSession::open` discards the flags parameter entirely and just
  calls `libseat::Seat::open_device` — so whether an `Action::Spawn`-launched
  child inherits DRM master or input-device fds depends entirely on
  libseat's own (unverified from this machine) C-side behavior, not on
  flexwm's request. Not independently confirmed either way; a one-command
  check if this ever matters:
  `grep flags /proc/<flexwm-pid>/fdinfo/<drm-fd-num>` (bit `02000000` =
  `O_CLOEXEC`) while `--tty` is running.
- **IPC socket path is unlink-then-bind (LOW, non-default config only).**
  `ipc.rs` does `remove_file` then `bind` — a symlink-race pattern in
  principle, only exploitable if `$FLEXWM_SOCKET` is overridden into a
  directory writable by another user (the default `$XDG_RUNTIME_DIR` path
  isn't).
- **Config parsing has no recursion-depth guard (LOW).** The `toml` stack
  has no explicit guard against deeply nested input; a maliciously deep
  config could stack-overflow-abort the process rather than hit the
  module's normal "log and fall back to defaults" path. Requires the user's
  own config file, so low priority.
- **`flexwm-core`'s `gap` config value has no upper bound (LOW).** Clamped
  only at the bottom (`.max(0)`); a very large configured gap can overflow
  plain `i32` arithmetic in `layout.rs`/`arrange.rs`. Config-only, same fix
  shape as the `min_size` finding above.
- **No upper bound on shm pool size (LOW).** Found while verifying item 7's
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

- **Custom/client cursor support — (a) DONE as item 8, (b) still open.**
  Item 5 shipped a fixed, procedurally-generated triangle for every
  `CursorImageStatus` variant — `Named` (a requested xcursor theme name) and
  `Surface` (a client-supplied cursor image, e.g. a text-input I-beam or a
  resize arrow) both drew the exact same shape, ignoring what was actually
  requested. ~~(a) honor `CursorImageStatus::Surface` by rendering the
  client's actual supplied buffer as the cursor element~~ — landed as item 8
  above. Still open: **(b) a user/config-level override (cursor theme name,
  size, or color in `config.toml`'s `[appearance]`-shaped section) for the
  fallback shape itself**, which is what `Named` will always fall back to —
  there is no client buffer behind a `Named` request, only a theme name, so
  (b) is the only thing that can ever make it look right. Keep the license
  constraint from `CLAUDE.md` in mind if (b) ever means loading a real
  xcursor theme (niri's assets are GPL, Adwaita's aren't MIT-clean).
