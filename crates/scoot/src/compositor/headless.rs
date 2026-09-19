//! The headless backend: draw into memory on the CPU, with no display at all.
//!
//! This is what agents and tests drive. Nothing is shown anywhere; the way to
//! see the screen is a screenshot over IPC.

use std::error::Error;
use std::time::{Duration, Instant};

use scoot_core::{Event as CoreEvent, OutputId, Rect};
use smithay::desktop::layer_map_for_output;
use smithay::desktop::utils::send_frames_surface_tree;
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::Transform;

use super::State;
use super::output_scale::smithay_scale;
use super::render::{self, Backend};
use super::session_lock::LOCK_VBLANK_TIMEOUT;
use super::tty::Tty;

/// The per-frame cost of [`State::render`], for the record CLAUDE.md asks
/// for. Printed, not asserted -- see the module's own doc.
#[cfg(test)]
mod bench;

/// The core's id for the one output this compositor creates.
///
/// Every site that reports output geometry to [`scoot_core`] uses it, so the
/// "there is exactly one output" assumption lives in one named place rather
/// than as a bare `OutputId(1)` repeated at each of them. Multi-output
/// support replaces this with a real per-output id.
///
/// It is not the only thing multi-output has to touch, though, so this is not
/// a "change the constant and you're done" marker. `layer_shell.rs`'s
/// `new_layer_surface` already honours a client's requested `wl_output`
/// (`layer_map_for_output(requested.or(self.output))`), while six sites
/// unconditionally reach for `self.output` instead: `layer_destroyed`,
/// `commit_layer_surface`, `refresh_layer_zone`, `layer_hit`,
/// `layer_keyboard_focus` and `render()`'s frame-callback/cleanup pass.
/// Unreachable today -- exactly one `Output` exists, so "the one the client
/// asked for" and "the one we have" are the same object -- but every one of
/// them has to become per-output once there is more than one, and not all in
/// the same way: `layer_destroyed` and `commit_layer_surface` want the
/// surface's *own* output, `layer_hit` the one under the pointer,
/// `refresh_layer_zone` a zone per output rather than one, and
/// `layer_keyboard_focus` and `render()`'s pass have to walk more than one
/// map.
///
/// The same holds for `session_lock.rs`, whose four per-output sites are the
/// subject of `docs/backlog/resolved/session-lock-per-output-done.md` (resolved
/// as single-output pins, not as multi-output): `new_surface` honours the
/// client's named `wl_output` but falls back to the single output, and
/// refuses a second live surface for an already-covered output with the
/// protocol's `duplicate_output` error (see
/// `docs/backlog/resolved/session-lock-duplicate-output-done.md`);
/// `configure_all` sizes every surface to that one output together;
/// confirmation treats the first blanked frame as "presented on all outputs",
/// which is true if and only if there is one of them; and the locked render
/// path composites every current surface onto that output at its origin with
/// the keyboard on the first of them. Per-output, the fallback goes away,
/// each output gets its own size, `locked` waits for every output's blanked
/// frame, and the focus rule needs one surface per output rather than the
/// first of all of them.
pub(super) const OUTPUT_ID: OutputId = OutputId(1);

/// How often a changed screen is redrawn, while there's something to redraw.
///
/// Visible to the rest of the compositor because it is also the natural unit
/// for "how much work may one client ask for": `ipc.rs` spaces a connection's
/// screenshots by it, so a capture costs at most what a frame already does.
/// Not a claim about the pixels -- `render()` runs on demand from
/// `needs_render`, not on frame boundaries, so two captures a frame apart can
/// legitimately differ.
pub(super) const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// The output's name -- and model -- when no backend has a better one:
/// `--headless` and `--nested`, and every test. `--tty` passes its
/// connector's name to [`init_named`] instead.
pub const OUTPUT_NAME: &str = "headless";

/// [`init_named`] under the default name -- what every test harness wants,
/// and the same output every test has always had. `compositor::run` goes
/// through `init_named` for all three backends (with [`OUTPUT_NAME`] for the
/// two connector-less ones), so this is test-only.
#[cfg(test)]
pub fn init(state: &mut State, width: i32, height: i32) -> Result<(), Box<dyn Error>> {
    init_named(state, OUTPUT_NAME, width, height)
}

/// Creates the one output and the CPU render target behind it. `name` is
/// what clients see as `wl_output.name` (and `model`): a connector name
/// such as `HDMI-A-1` or `Virtual-1` under `--tty`, so a bar or shell
/// labels the screen the way it would under any other compositor, or
/// [`OUTPUT_NAME`] where there is no connector.
pub fn init_named(
    state: &mut State,
    name: &str,
    width: i32,
    height: i32,
) -> Result<(), Box<dyn Error>> {
    let output = Output::new(
        name.to_owned(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "scoot".into(),
            model: name.to_owned(),
            serial_number: "0".into(),
        },
    );
    output.create_global::<State>(&state.display_handle);
    set_mode(
        &output,
        width,
        height,
        Some((0, 0).into()),
        state.output_scale,
    );
    state.space.map_output(&output, (0, 0));

    state.backend = Some(Backend::new(&output, width, height)?);
    // What the core is told is the *logical* output rectangle, which is the
    // same rectangle Smithay's `Space` lays windows out in -- see
    // `output_scale.rs`'s `logical_size`. Handing the core the physical
    // framebuffer size instead (what this did before output scaling existed)
    // makes every window, gap and edge land at physical coordinates the
    // render path then scales a second time.
    let area = state
        .space
        .output_geometry(&output)
        .map(|geometry| {
            Rect::new(
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            )
        })
        .unwrap_or_else(|| Rect::new(0, 0, width, height));
    state.output = Some(output);
    // The cursor's startup position: centred, not at the origin Smithay
    // leaves it at. Here rather than per-backend, so all three backends
    // place it at the same init point -- the cursor draws only under `--tty`
    // today, but the position is backend-independent seat state. See
    // `place_pointer_at_output_centre` for the quiet-path and no-replace
    // reasoning.
    state.place_pointer_at_output_centre();
    // The head a display-configuration client sees. After `state.output` is
    // set, because that is where the advertised state is read from -- and
    // before `apply()`, so a client that bound the manager during `State::new`
    // (before there was an output) hears about the head in the same startup
    // pass everything else is announced in. See `output_management.rs`.
    state.refresh_output_heads();
    state.world.handle_event(CoreEvent::OutputAdded {
        id: OUTPUT_ID,
        area,
    });
    // apply() ends in request_render(), which arms the frame timer via
    // ensure_ticking() -- this is what puts the very first frame on it.
    state.apply();

    Ok(())
}

/// Updates an already-created output's mode and scale. `location` is only
/// meaningful the first time (see `init`); later callers
/// (`State::resize_output`) pass `None` to leave it where it is.
///
/// The third `change_current_state` argument is the scale slot the pre-output-
/// scaling code passed `None` for, which left every output at Smithay's
/// default `Scale::Integer(1)`. See `output_scale.rs` for why exactly 1.0 is
/// still spelled `Scale::Integer(1)`.
fn set_mode(
    output: &Output,
    width: i32,
    height: i32,
    location: Option<smithay::utils::Point<i32, smithay::utils::Logical>>,
    scale: f64,
) {
    let mode = Mode {
        size: (width, height).into(),
        refresh: 60_000,
    };
    // `set_preferred` first: `change_current_state` sends the new mode to
    // already-bound `wl_output` clients synchronously, with the `preferred`
    // bit exactly when the mode it is told about already is the preferred
    // one (see `wl_change_current_state` at the pinned Smithay rev -- no
    // batching, call order is what the wire sees). The other order told a
    // client bound before a resize about the new mode with no `preferred`
    // bit, and nothing ever resent it; see
    // `docs/backlog/resolved/wl-output-preferred-flag-on-late-mode-done.md`.
    output.set_preferred(mode);
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        Some(smithay_scale(scale)),
        location,
    );
}

impl State {
    /// Draws a frame, if anything changed since the last one.
    pub fn render(&mut self) {
        if !self.needs_render {
            return;
        }
        // While the `--tty` session holds no DRM master (`Tty::active` is
        // `false` -- VT-switched away, or a reactivation whose `drm.activate`
        // itself failed), `present()` drops every frame, so rendering one and
        // waking clients with frame callbacks is pure waste: compositor CPU
        // plus clients painting frames nobody shows. Skip both.
        //
        // `needs_render` is cleared, not left set: nothing can be shown until
        // a successful `reactivate()`, and that path unconditionally asks for
        // a fresh render (`session_event`'s `ActivateSession` arm maps even
        // `Reconfigured::Nothing` to `Render`), so no repaint is lost. With
        // the flag cleared the frame timer drops itself instead of ticking
        // 60-odd times a second to find the session still paused, and the
        // compositor idles until a client commit or the switch back re-arms
        // it. Damage needs no special handling: neither the damage tracker
        // (no `render_output` call advances its history) nor the dumb-buffer
        // ages (no `advance_generation`) move while renders are skipped, and
        // `reactivate()` invalidates the ages, so the first post-reactivate
        // render is a full repaint.
        //
        // Gated on `active` (no DRM master), deliberately *not*
        // `session_paused`: the failed-reactivation state (session active
        // again, master not reacquired) drops frames in `present()` exactly
        // like a paused one, so rendering there is the same waste. See
        // `Tty::active`'s doc for why the two differ.
        //
        // What this defers, all harmlessly: a session that locks while paused
        // stays bare-`pending` with no deadline armed -- `await_vblank`,
        // the sole site arming `blank_deadline`, and the fallback timer
        // beside it both live in the render tail past this gate, so neither
        // runs -- and confirms on the reactivation render, via vblank or
        // the fallback, both of which are intact there. No wedge: the wait
        // simply waits. Note the timing delta on this security-sensitive
        // path: pre-PR the locker got `locked` via the fallback about a
        // second after requesting it, with no blanked frame on scanout;
        // now `locked` waits for the switch-back, when one actually reaches
        // scanout -- arguably more protocol-correct, but a behaviour change
        // worth naming, not implying away; popup grabs, IME composition and keyboard
        // focus are seat state, untouched by withholding frame callbacks, and
        // only their *pacing* pauses (the same shape as a minimized window in
        // most compositors: no protocol promises a callback by any deadline);
        // dead-layer/popup cleanup and `flush_clients` wait for the next live
        // render. An IPC screenshot taken while paused reads back the last
        // pre-pause framebuffer -- the last thing anyone saw.
        if tty_blocks_render(self.tty.as_ref().map(Tty::is_active)) {
            self.needs_render = false;
            return;
        }
        // Order matters: `backend.take()` must not run unless `output` is
        // also present, or a None output would leave it taken and never put
        // back -- silently and permanently losing the backend on the next
        // render attempt.
        let Some(output) = self.output.clone() else {
            return;
        };
        let Some(mut backend) = self.backend.take() else {
            return;
        };
        // Read once, here, so the element gathering, the clear colour and
        // the frame callbacks below are all answering the same question
        // about the same frame.
        let locked = self.session_lock.is_locked();
        // The frame itself, drawn with whichever renderer this session is
        // carrying. See `render.rs` for why the choice of renderer is an
        // enum dispatched once per frame rather than a type parameter
        // threaded through `State`.
        let frame = render::draw_frame(self, &mut backend, &output, locked);
        self.backend = Some(backend);
        self.needs_render = false;
        // A refused `--tty` flip (see `FrameOutcome::retry_render`): nothing is in
        // flight, so no `VBlank` will ever arrive to retry it the way
        // `present_skipped` retries an in-flight skip -- re-arm the frame
        // timer directly so the re-presented frame goes out on the next
        // tick instead of waiting for unrelated damage. After the flag
        // clear above, so this request survives it; bounded at the source
        // (`present_retry.rs`), so a device that keeps refusing goes quiet
        // instead of pinning the loop.
        if frame.retry_render {
            self.request_render();
        }

        // The frame a pending session lock has been waiting for. Only after
        // one has actually been drawn -- never after a failed bind or a
        // failed render, which would leave whatever was on screen before the
        // lock exactly where it was -- may the client be told `locked`. A
        // lock that could not be confirmed stays pending and stays *locked*;
        // the alternative, giving up and unlocking, would turn a renderer
        // failure into an unrequested unlock (see `session_lock.rs`).
        //
        // Where there is a scanout to wait for (`--tty`), confirmation
        // additionally waits for the vblank of the flip carrying the blanked
        // frame (see `SessionLock::await_vblank`): the previous, possibly
        // unlocked, frame can otherwise stay on scanout for up to one more
        // vblank after `locked` has gone out. Headless and nested have no
        // scanout, so the drawn frame *is* the shown one and confirms at
        // once, exactly as before.
        if frame.drew_a_frame {
            if self.session_lock.awaiting_blank() && self.tty.is_some() {
                let now = Instant::now();
                if self.session_lock.await_vblank(frame.blank_seq, now) {
                    // This frame needs a timer watching the fallback
                    // deadline: a newly armed wait has none yet, and a
                    // freshly issued flip restarted the bound out from
                    // under the previous one (see `await_vblank`). One
                    // shot each, dropped when they fire: the vblank path
                    // takes the wait first in the ordinary case, so a timer
                    // only ever fires for a vblank that never came -- and a
                    // stale one finds no wait and drops.
                    if let Err(error) = self
                        .loop_handle
                        .insert_source(Timer::from_duration(LOCK_VBLANK_TIMEOUT), blank_timeout)
                    {
                        // error!, not warn!: without this timer a lock whose
                        // vblank never arrives hangs its locker forever --
                        // the one failure mode this wait exists to prevent.
                        // The vblank path itself still works; only the
                        // fallback is gone.
                        tracing::error!(
                            %error,
                            "could not arm the session-lock vblank fallback timer; \
                             a lock whose vblank never arrives will hang its locker"
                        );
                    }
                }
            } else {
                self.confirm_lock();
            }
        }

        // Presentation feedback for the frame that just went out, before the
        // frame callbacks below (Smithay's ordering: drain feedback first,
        // then tell clients to draw next). Taken if and only if something
        // was actually shown: a `--tty` flip issued, a `--nested` commit
        // handed to the host, or -- with no presenter at all -- a frame
        // drawn into the framebuffer, which *is* the final image there. A
        // rendered-but-dropped frame (busy CRTC, no free host buffer, size
        // mismatch, failed bind or draw) leaves pending feedback queued for
        // the next presented frame rather than stamping a time nothing was
        // shown at; while locked only the lock surfaces are stamped (see
        // `presentation_time.rs` for the rule and the per-backend timestamp
        // semantics).
        //
        // `vsync` is the backend, not the flip: `--tty` page flips are
        // vblank-synchronized whenever one is issued, and this arm only runs
        // when one was.
        //
        // `seq` is per-backend per the protocol's `presented` contract (see
        // `presented_frame`): the issued-flip number on `--tty`, zero
        // everywhere else -- headless has no retrace to count and nested
        // output is self-refreshing with no queryable count.
        if let Some(seq) = super::presentation_time::presented_frame(
            self.host.is_some(),
            frame.host_committed,
            self.tty.is_some(),
            frame.blank_seq,
            frame.drew_a_frame,
        ) {
            self.present_feedback(
                &output,
                self.tty.is_some(),
                frame.cursor_surface.as_ref(),
                seq,
            );
        }

        let time = self.start_time.elapsed();
        // A client cursor surface is never in `self.space`, so the window
        // loop below can't reach it -- and a well-behaved client with an
        // animated cursor (a spinner, a throbber) attaches one frame,
        // requests a callback, and waits for it before attaching the next.
        // Without this it waits forever and the animation freezes on its
        // first frame.
        //
        // Scoped to the frames that presented this cursor (`--tty`, pointer
        // present, the status a live client surface) rather than sent
        // unconditionally: waking a client to draw cursor frames that
        // nothing on screen is showing is exactly the kind of pointless
        // work this compositor's own render loop avoids. It is deliberately
        // *not* narrowed further to "the surface produced at least one
        // element" -- a surface with no buffer yet produces none, and a
        // client that asks for a callback before its first attach (legal,
        // if unusual) would then stall on the very frame that should unstick
        // it.
        //
        // Outside the lock branch below because the cursor is drawn either
        // way, and while locked that surface can only belong to the lock
        // client: `wl_pointer.set_cursor` is refused unless the asking client
        // holds pointer focus or a pointer grab (Smithay's
        // `allow_setting_cursor`), and locking resets the image, drops grabs
        // and moves pointer focus onto the lock surface.
        if let Some(surface) = &frame.cursor_surface {
            send_frames_surface_tree(surface, &output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }
        if locked {
            // Lock surfaces are the only clients told to draw: "the
            // compositor must stop rendering and providing input to normal
            // clients". A client with no frame callback stops drawing by
            // itself, so this is both what the protocol asks for and what
            // keeps every window and bar in the session from burning CPU
            // behind a lock screen. They get one again on the first frame
            // after unlocking.
            if self.lock_post_frame(&output, time) {
                // A lock surface was dropped, and it may have been the one
                // holding the keyboard, the pointer or a grab -- and what is
                // drawn just changed. Pointer focus has to be re-derived
                // explicitly and not just hit-tested: `wl_pointer.button`
                // goes to whatever the pointer last entered, and a grab
                // outlives focus changes entirely (see `session_lock.rs`).
                self.lock_transition();
            }
        } else {
            for window in self.space.elements() {
                window.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                    Some(output.clone())
                });
            }
            // Layer surfaces aren't in `self.space` either, and a bar's clock
            // stops at whatever second it first drew without this -- the same
            // frame-callback starvation the cursor surface had. Sent to every
            // mapped layer surface rather than only the ones that produced an
            // element, matching both the window loop above and the cursor's own
            // reasoning: a client may legitimately ask for a callback before its
            // first attach, and withholding it would stall the very frame that
            // unsticks it.
            let dropped_dead_layers = {
                let mut layers = layer_map_for_output(&output);
                for layer in layers.layers() {
                    layer.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    });
                }
                // Second line of defence behind `layer_destroyed` (see
                // `layer_shell.rs`), for a client whose implicit teardown ran in
                // an order that left a dead surface mapped. `cleanup` only walks
                // the list -- no work at all with no layer surfaces -- and
                // re-arranges if it removed one, which is why the zone is
                // re-derived below when it did.
                let before = layers.len();
                layers.cleanup();
                before != layers.len()
            };
            if dropped_dead_layers {
                self.refresh_layer_zone();
                // One of those dead surfaces may have been holding the keyboard
                // (`layer_destroyed` is the usual path back, but this branch
                // exists precisely for the teardown orders it misses), and
                // `refresh_layer_zone` returns early when the zone didn't move.
                self.refresh_keyboard_focus();
            }
        }
        self.space.refresh();
        self.popups.cleanup();
        let _ = self.display_handle.flush_clients();
    }

    /// Recreates the render target at a new size. Returns whether it
    /// actually got there.
    ///
    /// Two callers: `--nested`, when the host's first configure disagrees
    /// with the size scoot started at (see `nested::init`), and `--tty`,
    /// when a DRM hotplug changes the connector's mode (see
    /// `tty/hotplug.rs`). This function doesn't know `Host` or `Tty` exists
    /// -- it only touches the render target and the core's notion of output
    /// geometry; the caller is responsible for resizing its own scanout
    /// buffers to match, separately.
    ///
    /// `false` means the render target could not be rebuilt and is still at
    /// the *old* size while the caller has (or is about to) size its scanout
    /// buffers to the new one. Both callers treat that as fatal-ish rather
    /// than something to limp on from, because the two sizes disagreeing is
    /// exactly what `present()`'s size guard silently drops every frame for:
    /// `--nested` stops the loop, `--tty` logs an error saying the screen
    /// stays as it is until the next hotplug or a restart. Nothing is
    /// reverted here -- a failure to build a pixman image at one size is not
    /// evidence that rebuilding it at the previous size would work.
    pub fn resize_output(&mut self, width: i32, height: i32) -> bool {
        let Some(output) = self.output.clone() else {
            // Logged, not a silent `false`: both callers' comments say
            // "`resize_output` has already logged what failed", and without
            // this that was only true of the `Backend::new` path below.
            // Unreachable today -- `headless::init_named` runs before either
            // backend can ask for a resize -- but a `false` nobody can
            // explain is exactly the shape of failure this project treats as
            // seriously as a crash.
            tracing::warn!(width, height, "could not resize: there is no output yet");
            return false;
        };
        set_mode(&output, width, height, None, self.output_scale);
        match Backend::new(&output, width, height) {
            Ok(backend) => self.backend = Some(backend),
            Err(error) => {
                tracing::warn!(%error, "could not resize the render target");
                return false;
            }
        }
        // The logical rectangle the core and the `Space` both work in -- see
        // `output_scale.rs`'s `logical_size`, and `init`'s comment on why the
        // core must never be handed the physical size.
        let logical = self
            .space
            .output_geometry(&output)
            .map(|geometry| (geometry.size.w, geometry.size.h))
            .unwrap_or((width, height));
        // A lock surface's configured size is an *exact* requirement -- the
        // next buffer that doesn't match it is a `dimensions_mismatch`
        // protocol error, i.e. a killed lock client on a locked session -- so
        // a resized output has to reconfigure them. A no-op when the session
        // isn't locked (there are none). Logical, not physical: a lock
        // surface's configure size is in surface-local logical coordinates.
        self.resize_lock_surfaces(logical);
        // Layer surfaces are anchored to the output's edges, so every one of
        // them has moved or resized -- `LayerMap::arrange` recomputes their
        // rectangles against the new mode and configures whoever needs a new
        // size. Before the core hears about the resize, so that
        // `refresh_layer_zone` below reports a zone measured against this
        // mode rather than the previous one.
        layer_map_for_output(&output).arrange();
        // The mode changed, so every output-management client is a mode, a
        // current-mode and a `done` behind. `set_mode` above is the write; this
        // is the only other site that reaches it (see `output_management.rs`).
        self.refresh_output_heads();
        // ...and every screen-capture client is holding a buffer sized for the
        // old framebuffer, which its next capture would be refused for with no
        // explanation. After the new backend is in place, because the size
        // re-advertised is read from it. See `screencopy.rs`.
        self.refresh_capture_constraints();
        self.world.handle_event(CoreEvent::OutputChanged {
            id: OUTPUT_ID,
            area: Rect::new(0, 0, logical.0, logical.1),
        });
        // The core re-clamps its old usable area into the new one on
        // `OutputChanged` (see `scoot_core`'s `Output::set_area`), which is
        // the right thing to do with a reservation nobody has re-reported
        // yet -- this is that re-report. It only does anything when the zone
        // actually moved (a bar is mapped, so the new mode leaves a
        // different usable rectangle); with nothing reserving anything it
        // returns early, because `OutputChanged`'s own re-clamp has already
        // left the core's usable area equal to the zone this recomputes.
        self.refresh_layer_zone();
        // `apply()`, not `request_render()`: the core now lays out against a
        // different rectangle, and nothing else pushes that arrangement onto
        // the windows -- `refresh_layer_zone` above ends in `apply()` only on
        // the paths where the zone moved, which is not the common case (no
        // bar mapped, or a bar whose exclusive zone is unchanged). Without
        // this, a window keeps the size it was configured at before the
        // resize and sits clipped or half off the new output until some
        // unrelated action happens to call `apply()`. That was harmless while
        // the only caller was `nested::apply_size` (one call per process, on
        // the host's first configure, before any window has mapped) and is
        // not once `--tty` resizes dynamically on hotplug with windows
        // already up. `apply()` ends in `request_render()`, so the frame is
        // still requested exactly once.
        self.apply();
        true
    }

    /// Marks the screen dirty and makes sure the frame ticker is running to
    /// actually redraw it.
    pub fn request_render(&mut self) {
        self.needs_render = true;
        self.ensure_ticking();
    }

    /// Arms the frame timer if it isn't already running.
    ///
    /// At idle -- nothing to redraw, no `wait-idle` outstanding -- the timer
    /// drops itself (see `frame_tick`) instead of polling 60-odd times a
    /// second forever, which is what this compositor did before: every tick
    /// woke the process just to find `needs_render` false and go back to
    /// sleep. This is the other half of that: whatever sets `needs_render` or
    /// registers a `PendingIdle` calls this to wake it back up.
    pub fn ensure_ticking(&mut self) {
        if self.timer_armed {
            return;
        }
        self.timer_armed = true;
        if let Err(error) = self
            .loop_handle
            .insert_source(Timer::from_duration(FRAME_INTERVAL), frame_tick)
        {
            // Rendering and wait-idle both depend on this timer; if it can't
            // be armed, both are now silently dead until something restarts
            // the process. That's worth a loud log even though there's no
            // Result to propagate up from here.
            tracing::error!(%error, "could not arm the frame timer");
            self.timer_armed = false;
        }
    }
}

fn frame_tick(_now: std::time::Instant, _metadata: &mut (), state: &mut State) -> TimeoutAction {
    state.render();
    // Immediately after the render, so a screen-capture client is handed the
    // frame that was just drawn rather than the one before it. Deliberately
    // *not* part of this tick's re-arm decision below: a capture session
    // waiting for the screen to change has nothing to do until something else
    // marks it dirty, and keeping the timer alive for it would undo the idle
    // behaviour `ensure_ticking` exists for. See `screencopy.rs`.
    state.service_captures();
    state.settle_idle_waiters();
    state.settle_shots();
    if state.needs_render || !state.pending_idle.is_empty() || state.shots_draining() {
        TimeoutAction::ToDuration(FRAME_INTERVAL)
    } else {
        state.timer_armed = false;
        TimeoutAction::Drop
    }
}

/// Fires [`LOCK_VBLANK_TIMEOUT`] after a blanked frame rendered under `--tty`
/// without its vblank confirming the lock: the fallback half of the vblank
/// wait (see `SessionLock::await_vblank`). One shot, always dropped -- a
/// re-presented flip arms its own timer against its own restarted bound, so
/// a timer only ever fires for a vblank that never came, and a stale one
/// finds no live deadline and drops. `Instant::now()` rather than the
/// timer's own timestamp, so the bound is measured against the same clock
/// the wait was armed with.
fn blank_timeout(_now: std::time::Instant, _metadata: &mut (), state: &mut State) -> TimeoutAction {
    state.note_blank_timeout(Instant::now());
    TimeoutAction::Drop
}

/// Whether `render()` must skip this frame because no presenter can show
/// it: `Some(false)` is a `--tty` session holding no DRM master (paused or a
/// failed reactivation -- see `Tty::active`), `Some(true)` is one holding it,
/// `None` is every other backend (headless, nested, and every test harness,
/// none of which has a `Tty` at all).
///
/// A free function over the flag rather than a method on `Tty` so the truth
/// table is unit-testable: no test harness can construct a `Tty` (it needs a
/// live DRM device), so a method could only ever be pinned live on `--tty`
/// hardware. The `map(Tty::is_active)` call at the `render()` gate is the
/// only caller.
fn tty_blocks_render(tty_active: Option<bool>) -> bool {
    tty_active.is_some_and(|active| !active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_tty_never_blocks_a_render() {
        // Headless, nested, and every test harness: no `Tty` exists, so the
        // gate is a no-op and the whole existing suite pins the unblocked
        // path without knowing about this predicate.
        assert!(!tty_blocks_render(None));
    }

    #[test]
    fn an_active_tty_never_blocks_a_render() {
        assert!(!tty_blocks_render(Some(true)));
    }

    #[test]
    fn a_masterless_tty_blocks_a_render() {
        // Paused (VT-switched away) and failed-reactivation alike: both mean
        // `present()` drops every frame, so there is nothing a render could
        // show. This is the fail-first pin for the gate -- negate the
        // predicate body and this fails.
        assert!(tty_blocks_render(Some(false)));
    }
}
