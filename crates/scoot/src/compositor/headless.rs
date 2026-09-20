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
use super::render::{self, Backend, ScanoutHandoff};
use super::session_lock::LOCK_VBLANK_TIMEOUT;
use super::tty::Tty;

/// The per-frame cost of [`State::render`], for the record CLAUDE.md asks
/// for. Printed, not asserted -- see the module's own doc.
#[cfg(test)]
mod bench;

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
    init_named(state, OUTPUT_NAME, width, height, ScanoutHandoff::default())
}

/// Creates the primary output and the render target behind it -- pixman's
/// image or, under `--renderer gles`, a GLES renderbuffer (see `render`).
/// `name` is what clients see as `wl_output.name` (and `model`): a connector
/// name such as `HDMI-A-1` or `Virtual-1` under `--tty`, so a bar or shell
/// labels the screen the way it would under any other compositor, or
/// [`OUTPUT_NAME`] where there is no connector.
///
/// The output this creates is the one [`Outputs::primary`] hands back, and
/// the only one with a [`Backend`] behind it; [`add_output`] puts further
/// headless outputs beside it and refuses to run before it.
///
/// [`Outputs::primary`]: super::outputs::Outputs::primary
pub fn init_named(
    state: &mut State,
    name: &str,
    width: i32,
    height: i32,
    scanout: ScanoutHandoff,
) -> Result<(), Box<dyn Error>> {
    let output = create_output(state, name, width, height, (0, 0));

    // The renderer the session resolved at startup (`render::resolve`, then
    // `tty::init`'s own fallback), not a per-call choice:
    // `State::resize_output` rebuilds the backend later and has to build the
    // same one. `scanout` is the already-built GPU scanout renderer on the
    // one path that has one (see `ScanoutHandoff`).
    let backend = Backend::new(&output, width, height, state.renderer, scanout)?;
    // The one place the dmabuf advertisement can be made, and the reason it is
    // here rather than in `State::new` with every other global: the tranche is
    // derived from what *this* renderer can import (see `dmabuf.rs`), and this
    // is the first instant a renderer exists. Nothing can observe the
    // difference -- the wayland listening socket is a calloop source, so no
    // connection is accepted, no registry served and no global announced until
    // `event_loop.run`, which is several steps after this on every path
    // (`compositor::run`) and after the client is even spawned in every test
    // harness.
    super::dmabuf::advertise(
        &state.display_handle,
        &mut state.screencopy.dmabuf,
        &backend,
    );
    // One backend, one output, and `state.backend` is *replaced* while
    // `state.outputs` is *appended to* -- so calling this twice would leave
    // `primary()` naming output 1 while the backend belongs to output 2.
    // Every capture and gamma path compares against `primary()`, so they
    // would then refuse the one output that actually has pixels, and
    // `render()` would draw output 1's geometry into output 2's target.
    // Unreachable today (one call, at startup) and cheap to keep that way;
    // `Outputs::add` got a structural guard for the same reason.
    debug_assert!(
        state.outputs.is_empty(),
        "the headless backend was initialised twice: the second output would \
         be appended while the backend is replaced, leaving primary() and the \
         backend naming different outputs"
    );
    state.backend = Some(backend);
    // The GPU scanout tier's `DrmCompositor` was built before this output
    // existed (`tty::init` runs first, because `--tty` is where the size
    // comes from), so it is still tracking a static copy of the mode. Point
    // it at the real output now, while nothing has been drawn: from here it
    // follows every `set_mode` on its own, and no second place has to
    // remember to mirror a mode or scale change into it. A no-op on every
    // other backend and tier.
    if let Some(tty) = &mut state.tty {
        tty.track_output(&output);
    }
    let area = logical_area(state, &output, width, height);
    let id = state.outputs.add(output);
    // The cursor's startup position: centred, not at the origin Smithay
    // leaves it at. Here rather than per-backend, so all three backends
    // place it at the same init point -- the cursor draws only under `--tty`
    // today, but the position is backend-independent seat state. See
    // `place_pointer_at_output_centre` for the quiet-path and no-replace
    // reasoning.
    state.place_pointer_at_output_centre();
    // The head a display-configuration client sees. After the output is
    // registered, because that is where the advertised state is read from --
    // and before `apply()`, so a client that bound the manager during
    // `State::new` (before there was an output) hears about the head in the
    // same startup pass everything else is announced in. See
    // `output_management.rs`.
    state.refresh_output_heads();
    state
        .world
        .handle_event(CoreEvent::OutputAdded { id, area });
    // apply() ends in request_render(), which arms the frame timer via
    // ensure_ticking() -- this is what puts the very first frame on it.
    state.apply();

    Ok(())
}

/// Adds another headless output immediately to the right of the last one, and
/// hands back the core id it was filed under.
///
/// This is what `--headless --outputs N` builds outputs 2..N with (see
/// `cli.rs`). It creates a real `wl_output` global, maps the output into the
/// `Space` and tells the core about it, so a client can address it and the
/// core gives it its own scrolling strip -- but it deliberately builds **no
/// render target**: this compositor composites one framebuffer, the primary
/// output's, and making the render loop per-output is the multi-output item
/// rather than this one. Nothing is shown on a headless output in any case.
///
/// Refuses before [`init_named`] has run rather than trusting `run`'s call
/// order: the invariant that [`Outputs::primary`] is the output with the
/// backend is what lets a capture, a gamma ramp or a screenshot refuse any
/// *other* output instead of quietly answering from the wrong framebuffer.
///
/// [`Outputs::primary`]: super::outputs::Outputs::primary
pub fn add_output(
    state: &mut State,
    name: &str,
    width: i32,
    height: i32,
) -> Result<OutputId, Box<dyn Error>> {
    if state.outputs.is_empty() || state.backend.is_none() {
        return Err("an additional output needs the primary one to exist first".into());
    }
    // Measured off the previous output's *logical* geometry, not off `width`:
    // at `[output] scale = 2` a 1600px mode is 800 logical pixels wide, and
    // stepping by the physical width would leave a gap between the two
    // outputs that no pointer coordinate belongs to. `Space` and the core
    // both work in logical coordinates, so this is the one that makes them
    // adjacent.
    //
    // Saturating because the sum is client-independent but not
    // config-independent: `MAX_OUTPUTS` modes of `MAX_OUTPUT_DIMENSION` at the
    // `[output] scale` floor come to ~1e6, four orders inside `i32`, and a
    // saturated edge would merely stack two outputs rather than wrap into
    // negative coordinates.
    let x = state
        .outputs
        .last()
        .and_then(|previous| state.space.output_geometry(previous))
        .map(|geometry| geometry.loc.x.saturating_add(geometry.size.w))
        .unwrap_or(0);
    let output = create_output(state, name, width, height, (x, 0));
    let area = logical_area(state, &output, width, height);
    let id = state.outputs.add(output);
    state
        .world
        .handle_event(CoreEvent::OutputAdded { id, area });
    // The core lays out against one more output now, and `apply()` is what
    // pushes that arrangement onto the windows; it ends in `request_render()`.
    state.apply();
    Ok(id)
}

/// The `wl_output` half of creating an output: the global, its mode and scale,
/// and its place in the `Space`. Registering it with [`State::outputs`] and
/// telling the core about it are the caller's, because the two callers do
/// different work in between (see [`init_named`]).
///
/// [`State::outputs`]: super::State::outputs
fn create_output(
    state: &mut State,
    name: &str,
    width: i32,
    height: i32,
    position: (i32, i32),
) -> Output {
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
        Some(position.into()),
        state.output_scale,
    );
    state.space.map_output(&output, position);
    output
}

/// What the core is told an output covers: the *logical* rectangle, which is
/// the same one Smithay's `Space` lays windows out in -- see
/// `output_scale.rs`'s `logical_size`. Handing the core the physical
/// framebuffer size instead (what this did before output scaling existed)
/// makes every window, gap and edge land at physical coordinates the render
/// path then scales a second time.
///
/// The fallback is for an output the `Space` does not have, which cannot
/// happen on either path here (both map it first) and would otherwise be a
/// silent zero-sized output.
fn logical_area(state: &State, output: &Output, width: i32, height: i32) -> Rect {
    state
        .space
        .output_geometry(output)
        .map(|geometry| {
            Rect::new(
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            )
        })
        .unwrap_or_else(|| Rect::new(0, 0, width, height))
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
        // A host resize queued since the last tick is applied here, at most
        // one per frame, before anything else: this is the drain half of
        // `--nested`'s configure coalescing (see
        // [`Host::drain_pending_resize`](super::nested::Host::drain_pending_resize)).
        // Ahead of the clean-screen early return below, so a queued resize
        // is acted on even when nothing else dirtied the screen. A resize
        // applied here draws in this same frame (`apply_resize` ends in
        // `request_render`, and the frame below is composited after this).
        // A no-op on every backend without a host (`None` there), which is
        // every test harness: nothing outside `--nested` can queue one.
        super::nested::Host::drain_pending_resize(self);
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
        // The primary output, which is the one this framebuffer shows (see
        // `Outputs::primary`): a second `--outputs` output has no render
        // target of its own, and drawing it is the multi-output item.
        //
        // Order matters: `backend.take()` must not run unless the output is
        // also present, or a None output would leave it taken and never put
        // back -- silently and permanently losing the backend on the next
        // render attempt.
        let Some(output) = self.outputs.primary().cloned() else {
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
    /// Two callers: `--nested`, whenever the host configures scoot's window
    /// to a size it is not already at (its first configure, and every resize
    /// after -- see `nested::Host`), and `--tty`, when a DRM hotplug changes
    /// the connector's mode (see `tty/hotplug.rs`). This function doesn't
    /// know `Host` or `Tty` exists -- it only touches the render target and
    /// the core's notion of output geometry; the caller is responsible for
    /// resizing its own scanout buffers to match, separately.
    ///
    /// `false` means the render target could not be rebuilt and is still at
    /// the *old* size. The callers differ on what to do about that, and each
    /// says why at its own site: `--nested` stops the loop on a first
    /// configure and keeps the session at the old size on a resize (see
    /// `Host::apply_first_configure` and `Host::apply_resize`), `--tty` logs
    /// an error saying the screen stays as it is until the next hotplug or a
    /// restart. What this function owes all of them is that `false` means
    /// *nothing changed*: the render target is untouched, and the mode this
    /// had already advertised is put back, so no client is left believing a
    /// size nothing renders at. The render target is not rebuilt at the old
    /// size to get there -- it was never torn down -- which is why the
    /// restore cannot itself fail.
    ///
    /// It rebuilds whichever renderer the session started with
    /// (`State::renderer`), never a different one: a resize that silently
    /// changed renderers would be a session quietly different from the one
    /// that was asked for. Under `--renderer gles` that means a whole new
    /// EGL context and shader set per resize, which is wasteful and correct.
    /// What bounds it is the callers: `--tty`'s hotplug handler does not
    /// reach here unless the connector's mode actually changed, and
    /// `--nested` queues a configure's size and drains at most one per frame
    /// tick (see `Host::drain_pending_resize`) -- so this runs once per
    /// *drained* size, not once per event. A host window dragged to resize
    /// still pays it per frame the drag spans while the size keeps moving,
    /// which is the honest per-frame-budget cost; `--renderer gles` under
    /// `--nested` is opt-in, and resizing a live GLES target in place
    /// instead of rebuilding is the fix if that ever matters.
    pub fn resize_output(&mut self, width: i32, height: i32) -> bool {
        // Both halves in one read, so the `OutputChanged` below names the id
        // of the output that was actually resized without a second lookup
        // that could come back empty. Only the primary output can be resized:
        // its two callers are the `--nested` host configure and the `--tty`
        // hotplug, and both backends have exactly one output.
        let Some((id, output)) = self
            .outputs
            .primary_entry()
            .map(|(id, output)| (id, output.clone()))
        else {
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
        // Kept for the failure path below: `set_mode` tells every bound
        // `wl_output` client the new mode synchronously, so a `Backend::new`
        // that then fails would otherwise leave them believing a size nothing
        // will ever render at -- permanently, since nothing resends it. It is
        // only the advertised metadata that is put back, never the render
        // target (which was not torn down), so the restore cannot fail the
        // way rebuilding at the old size could.
        let previous = output.current_mode();
        set_mode(&output, width, height, None, self.output_scale);
        // The GPU scanout tier is resized, never rebuilt. Its `DrmCompositor`
        // tracks this same `Output` (see `Tty::track_output`), so `set_mode`
        // above has already moved its damage tracker onto the new mode, and
        // `Tty::reconfigure` has already resized its swapchain through
        // `DrmCompositor::use_mode`. Rebuilding would additionally throw away
        // a live EGL context, its shaders and its texture cache, plus the
        // frame in flight, to arrive at exactly the same pipeline. What is
        // left is the recorded framebuffer size, which capture clients read.
        if self
            .backend
            .as_ref()
            .is_some_and(super::render::Backend::is_scanout)
        {
            if let Some(backend) = &mut self.backend {
                backend.note_resized(width, height);
            }
        } else {
            match Backend::new(
                &output,
                width,
                height,
                self.renderer,
                ScanoutHandoff::default(),
            ) {
                Ok(backend) => self.backend = Some(backend),
                Err(error) => {
                    tracing::warn!(%error, "could not resize the render target");
                    // Back to the mode that is actually being rendered. The
                    // new one stays in `Output::modes` (that list only grows
                    // -- see `output_management.rs`), and a client that was
                    // told about it sees it become current and then current
                    // again at the old size; what it does not see is a
                    // current mode that disagrees with every frame it is
                    // sent. `None` when there is no previous mode is the
                    // never-resized-before case, which cannot reach here:
                    // `init_named` sets one before any caller exists.
                    if let Some(previous) = previous {
                        set_mode(
                            &output,
                            previous.size.w,
                            previous.size.h,
                            None,
                            self.output_scale,
                        );
                        // ...and the size that never rendered is taken back
                        // out of `Output::modes`, so a client binding later
                        // never hears about it and the next
                        // `refresh_output_heads` does not mint a
                        // `zwlr_output_mode_v1` for it. `set_mode` above
                        // pushed it twice over (once via `set_preferred`,
                        // once via `change_current_state`), and nothing
                        // prunes that list on its own -- without this every
                        // failed resize would leave a mode behind for the
                        // life of the session, announced to whoever binds
                        // next. Guarded on differing: a same-size call names
                        // the mode that is still current and preferred, and
                        // `delete_mode` clears both when they match.
                        //
                        // What this cannot take back is what an already-bound
                        // `wl_output` client was told synchronously, before
                        // the build failed: it saw the failed size become
                        // current *and* preferred, then the old size become
                        // current and preferred again, and `wl_output` has
                        // no un-prefer and no mode withdrawal to unsay the
                        // first half with. That transient is inherent to
                        // advertising before building (see `set_mode`'s
                        // caller order, which `wl_output`'s synchronous send
                        // forces), and it only opens on a resize that fails
                        // -- a pool that would not allocate -- not on the
                        // steady path.
                        let failed = Mode {
                            size: (width, height).into(),
                            refresh: 60_000,
                        };
                        if failed != previous {
                            output.delete_mode(failed);
                        }
                    }
                    return false;
                }
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
            id,
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
        // unrelated action happens to call `apply()`. That was harmless back
        // when the only caller was `--nested`'s first configure (one call per
        // process, before any window had mapped) and is not once `--tty`
        // resizes dynamically on hotplug -- nor once `--nested` follows a
        // host resize -- with windows already up. `apply()` ends in
        // `request_render()`, so the frame is still requested exactly once.
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
    use scoot_core::Config;
    use smithay::reexports::calloop::EventLoop;
    use smithay::reexports::wayland_server::Display;

    use super::*;
    use crate::compositor::decorations::Appearance;
    use crate::compositor::keybindings::Keybindings;

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

    /// A size no renderer can build a target for. pixman refuses it without
    /// allocating anything (`_pixman_multiply_overflows_int(width, 32)` in
    /// its own `create_bits`), and GLES refuses it past
    /// `GL_MAX_RENDERBUFFER_SIZE` -- so this drives `resize_output`'s failure
    /// path under either renderer the suite runs with.
    const UNBUILDABLE: (i32, i32) = (i32::MAX, 1);

    /// A failed resize must not leave clients believing a mode nothing
    /// renders at.
    ///
    /// `set_mode` tells every bound `wl_output` client synchronously, before
    /// the render target is rebuilt, and nothing resends it afterwards -- so
    /// a `Backend::new` failure after that point used to publish a size the
    /// compositor would never draw. It mattered less while every caller
    /// treated `false` as fatal; `--nested` now keeps the session running
    /// after a failed resize (see `Host::apply_resize`), which is exactly
    /// the case that would have to live with it. Fail-first: drop the
    /// restore in `resize_output` and this reports `2147483647x1`.
    #[test]
    fn a_failed_resize_leaves_the_advertised_mode_where_it_was() {
        const CANVAS: i32 = 200;
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
            super::super::test_support::test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        assert!(
            !state.resize_output(UNBUILDABLE.0, UNBUILDABLE.1),
            "a render target was somehow built at {UNBUILDABLE:?}"
        );

        // The primary output, which is the only one `resize_output` touches
        // (see its own doc) -- and, under the backends that reach it, the
        // only one there is.
        let mode = state
            .outputs
            .primary()
            .expect("an output")
            .current_mode()
            .expect("a current mode");
        assert_eq!(
            (mode.size.w, mode.size.h),
            (CANVAS, CANVAS),
            "a failed resize left the output advertising a mode nothing renders at"
        );
    }

    /// The other half of the same guarantee: the render target itself is
    /// untouched, so the session keeps drawing at the size it was already at
    /// rather than at neither size.
    #[test]
    fn a_failed_resize_leaves_the_render_target_where_it_was() {
        const CANVAS: i32 = 200;
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
            super::super::test_support::test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        assert!(!state.resize_output(UNBUILDABLE.0, UNBUILDABLE.1));

        assert_eq!(
            state.backend.as_ref().expect("a backend").size(),
            (CANVAS, CANVAS),
            "a failed resize replaced the render target with one at another size"
        );
    }

    /// The third half: the size that never rendered is taken back out of
    /// `Output::modes`, so a client binding later never learns it and the
    /// next refresh mints no `zwlr_output_mode_v1` for it. Fail-first: drop
    /// the `delete_mode` on the failure path and this reports two modes.
    #[test]
    fn a_failed_resize_leaves_no_mode_behind() {
        const CANVAS: i32 = 200;
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
            super::super::test_support::test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        assert!(!state.resize_output(UNBUILDABLE.0, UNBUILDABLE.1));

        let modes = state.outputs.primary().expect("an output").modes();
        assert_eq!(
            modes
                .iter()
                .map(|mode| (mode.size.w, mode.size.h))
                .collect::<Vec<_>>(),
            vec![(CANVAS, CANVAS)],
            "a failed resize left a mode behind that nothing renders at"
        );
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
