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
use smithay::utils::{Logical, Point, Transform};

use super::State;
use super::output_identity::OutputIdentity;
use super::output_scale::smithay_scale;
use super::reconnect::DisplacedOutput;
use super::render::{self, Backend, InPlace, ScanoutHandoff};
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
    init_named(state, OUTPUT_NAME, width, height, ScanoutHandoff::default()).map(|_| ())
}

/// Creates the primary output and the render target behind it -- pixman's
/// image or, under `--renderer gles`, a GLES renderbuffer (see `render`).
/// `name` is what clients see as `wl_output.name` (and `model`): a connector
/// name such as `HDMI-A-1` or `Virtual-1` under `--tty`, so a bar or shell
/// labels the screen the way it would under any other compositor, or
/// [`OUTPUT_NAME`] where there is no connector.
///
/// The output this creates is the one [`Outputs::primary`] hands back;
/// [`add_output`] puts further outputs beside it -- each with its own
/// [`Backend`] -- and refuses to run before it. Returns the id the output was
/// registered under, which `--tty` binds the connector's head to
/// (`tty::attach`).
///
/// [`Outputs::primary`]: super::outputs::Outputs::primary
pub fn init_named(
    state: &mut State,
    name: &str,
    width: i32,
    height: i32,
    scanout: ScanoutHandoff,
) -> Result<OutputId, Box<dyn Error>> {
    let output = create_output(state, name, width, height, (0, 0));

    // The renderer the session resolved at startup (`render::resolve`, then
    // `tty::init`'s own fallback), not a per-call choice:
    // `State::resize_output` rebuilds the backend later and has to build the
    // same one. `scanout` is the already-built GPU scanout renderer on the
    // one path that has one (see `ScanoutHandoff`).
    let backend = Backend::new(
        &output,
        width,
        height,
        state.renderer,
        scanout,
        state.gles_device(),
    )?;
    // The one place the dmabuf advertisement can be made, and the reason it is
    // here rather than in `State::new` with every other global: the tranche is
    // derived from what *this* renderer can import (see `dmabuf.rs`), and this
    // is the first instant a renderer exists. Nothing can observe the
    // difference -- the wayland listening socket is a calloop source, so no
    // connection is accepted, no registry served and no global announced until
    // `event_loop.run`, which is several steps after this on every path
    // (`compositor::run`) and after the client is even spawned in every test
    // harness.
    let advertised = super::dmabuf::advertise(
        &state.display_handle,
        &mut state.screencopy.dmabuf,
        &backend,
    );
    // Kept for the GPU scanout tier's per-surface feedback, which extends
    // and reverts to exactly what the global advertises (`dmabuf/scanout.rs`);
    // nothing else reads it.
    #[cfg(feature = "gpu-scanout")]
    {
        state.dmabuf_default = advertised;
    }
    #[cfg(not(feature = "gpu-scanout"))]
    drop(advertised);
    // One backend per output, and `state.backends` is keyed by the same id
    // `state.outputs` hands out -- so calling this twice would hand out a
    // second id with a second backend, and nothing names the wrong one.
    // Unreachable today (one call, at startup) and cheap to keep that way.
    debug_assert!(
        state.outputs.is_empty(),
        "the headless backend was initialised twice"
    );
    // The GPU scanout tier's `DrmCompositor` is pointed at this output by
    // `tty::attach`, which `compositor::run` calls with the id this returns
    // -- before the event loop starts, so still before anything is drawn.
    let area = logical_area(state, &output, width, height);
    let id = state.outputs.add(output);
    state.backends.insert(id, backend);
    // Name-only: the connector-less backends (and every test) identify by
    // name alone. `--tty` upgrades this to the full connector identity once
    // it knows it (`State::note_output_identity`); the primary is the first
    // output, so nothing displaced can be waiting for it and no restore runs
    // here (unlike `add_output_with` below).
    state
        .output_identities
        .insert(id, OutputIdentity::named(name));
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

    Ok(id)
}

/// Adds another headless output immediately to the right of the last one, and
/// hands back the core id it was filed under.
///
/// This is what `--headless --outputs N` builds outputs 2..N with (see
/// `cli.rs`). It creates a real `wl_output` global, maps the output into the
/// `Space` and tells the core about it, so a client can address it and the
/// core gives it its own scrolling strip -- plus a render target of its own,
/// built with the session's renderer at the same size, so the render loop
/// composites every output's own strip.
///
/// Refuses before [`init_named`] has run rather than trusting `run`'s call
/// order: a later output must never become the primary, and there must never
/// be an output with no render target for a capture to be answered from
/// another one's framebuffer.
///
/// [`Outputs::primary`]: super::outputs::Outputs::primary
pub fn add_output(
    state: &mut State,
    name: &str,
    width: i32,
    height: i32,
) -> Result<OutputId, Box<dyn Error>> {
    add_output_with(state, name, width, height, ScanoutHandoff::default())
}

/// [`add_output`], with the render target taking over `scanout` when it
/// carries one: the GPU scanout renderer a `--tty` head built alongside its
/// `DrmCompositor` (see [`ScanoutHandoff`]). What `--tty` creates every
/// output after the primary with, at startup (`compositor::run`) and when a
/// connector is plugged in (`tty/hotplug.rs`). With an empty handoff this is
/// exactly [`add_output`].
pub fn add_output_with(
    state: &mut State,
    name: &str,
    width: i32,
    height: i32,
    scanout: ScanoutHandoff,
) -> Result<OutputId, Box<dyn Error>> {
    if state.outputs.is_empty() || state.backends.is_empty() {
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
    // The render target behind this output: same renderer, same size as the
    // primary's. Built before the output is registered with `State::outputs`
    // or the core, and a failure takes back the two things `create_output`
    // already did -- the `wl_output` global and the `Space` mapping -- so
    // nothing is left half-added: the output never exists without its
    // target, and no capture path can resolve one and answer from another's.
    //
    // On the primary's GLES device, never another (`State::gles_device`): the
    // dma-buf formats advertised to clients are that device's, and an import
    // into this output's renderer must be able to honour them. A device that
    // cannot build here fails the add.
    let backend = match Backend::new(
        &output,
        width,
        height,
        state.renderer,
        scanout,
        state.gles_device(),
    ) {
        Ok(backend) => backend,
        Err(error) => {
            discard_output(state, &output);
            return Err(error);
        }
    };
    let area = logical_area(state, &output, width, height);
    let id = state.outputs.add(output);
    state.backends.insert(id, backend);
    // Name-only, like the primary above (`--tty` upgrades it after). Then
    // the restore: an output added under an identity a removed output filed
    // gets that output's still-open windows back (`State::restore_displaced`
    // is a no-op when nothing was filed).
    state
        .output_identities
        .insert(id, OutputIdentity::named(name));
    state
        .world
        .handle_event(CoreEvent::OutputAdded { id, area });
    // The head a display-configuration client sees for this output. `apply()`
    // below reaches the workspace groups through `refresh_workspaces`, but
    // output heads refresh only from here (and `init_named` and
    // `resize_output`): without this, a manager bound before this output
    // existed would never hear about it.
    state.refresh_output_heads();
    // The core lays out against one more output now, and `apply()` is what
    // pushes that arrangement onto the windows; it ends in `request_render()`.
    state.apply();
    state.restore_displaced(id);
    Ok(id)
}

/// An output with no render target, at the origin: in the `Space`, in
/// [`State::outputs`] and in the core, so windows are placed on it and the
/// pointer finds them there, but nothing draws it (`render()` skips an
/// output with no backend). For input suites on a bare harness that have no
/// use for a renderer -- they still need a real output, because a window
/// takes input only on the output it is placed on (see `output_clip.rs`).
///
/// [`State::outputs`]: super::State::outputs
#[cfg(test)]
pub(crate) fn add_output_without_backend(
    state: &mut State,
    name: &str,
    width: i32,
    height: i32,
) -> OutputId {
    let output = create_output(state, name, width, height, (0, 0));
    let area = logical_area(state, &output, width, height);
    let id = state.outputs.add(output);
    // Name-only, like the other add paths, and the restore: a harness output
    // added under a removed output's name gets its windows back the same way
    // a hotplugged monitor does.
    state
        .output_identities
        .insert(id, OutputIdentity::named(name));
    state
        .world
        .handle_event(CoreEvent::OutputAdded { id, area });
    state.restore_displaced(id);
    id
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
    let global = output.create_global::<State>(&state.display_handle);
    // Kept on the output itself, so the one path that must take an output
    // back before it is registered (`discard_output`) can find its global.
    output
        .user_data()
        .insert_if_missing(|| OutputGlobal(global));
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

/// The `wl_output` global [`create_output`] made for an output.
struct OutputGlobal(smithay::reexports::wayland_server::backend::GlobalId);

/// Takes back what [`create_output`] did, for an output that was never
/// registered: unmaps it from the `Space` and retires its `wl_output` global
/// (see [`retire_global`]). [`add_output_with`]'s failure path -- at startup,
/// and at runtime when a `--tty` connector is plugged in and its render
/// target cannot be built.
fn discard_output(state: &mut State, output: &Output) {
    state.space.unmap_output(output);
    retire_global(state, output);
}

/// How long a withdrawn `wl_output` global stays bindable before it is
/// destroyed. A client whose `wl_registry.bind` for it was already in flight
/// when the output went away must find the global still there (binding a
/// destroyed global is a protocol error that kills the client); a few seconds
/// covers any bind racing the `global_remove` the withdrawal sends. The same
/// shape niri and wlroots use. Shorter under test, so a suite can watch the
/// destruction happen without sleeping for five seconds -- but still far
/// longer than any racing-bind test takes to land its bind (a few
/// milliseconds of dispatch), so that test cannot turn flaky.
#[cfg(not(test))]
const RETIRED_GLOBAL_GRACE: Duration = Duration::from_secs(5);
#[cfg(test)]
const RETIRED_GLOBAL_GRACE: Duration = Duration::from_millis(500);

/// Withdraws `output`'s `wl_output` global now and destroys it after
/// [`RETIRED_GLOBAL_GRACE`] -- the only removal that is safe while clients
/// are connected (see the constant). Before the event loop runs (startup)
/// the timer simply fires later, which is equally safe.
///
/// A timer that cannot be registered leaves the global withdrawn but alive
/// for the session: no new client sees it, a bound one keeps a resource
/// with nothing behind it, and nothing is killed -- the safe way to fail.
fn retire_global(state: &mut State, output: &Output) {
    let Some(OutputGlobal(global)) = output.user_data().get::<OutputGlobal>() else {
        return;
    };
    let global = global.clone();
    state.display_handle.disable_global::<State>(global.clone());
    let timer = Timer::from_duration(RETIRED_GLOBAL_GRACE);
    let registered = state.loop_handle.insert_source(timer, move |_, _, state| {
        state.display_handle.remove_global::<State>(global.clone());
        TimeoutAction::Drop
    });
    if let Err(error) = registered {
        tracing::warn!(
            %error,
            "could not schedule a removed output's wl_output global for destruction; \
             it stays withdrawn but alive"
        );
    }
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
        // One framebuffer per output, drawn in creation order (so the primary
        // is first). Each output composites its own strip into its own render
        // target -- never another output's -- which is what keeps one
        // screen's pixels from being served as another's (see
        // `State::backends`).
        //
        // Walked by index rather than by iterator: drawing takes `&mut State`
        // (see `draw_frame`), which no borrow of `self.outputs` can outlive,
        // so each step clones its `(id, output)` -- an `Arc` bump, no
        // allocation -- and releases the borrow before drawing. The count is
        // read once up front; outputs are added and removed only by startup
        // and the `--tty` hotplug handler, never mid-frame, so it cannot go
        // stale inside this loop.
        //
        // Order matters per step the same way it used to for the single
        // output: a missing backend skips its output without touching the
        // others, and a taken backend is always put back before the next
        // output draws -- silently losing one would wedge that output's
        // captures and gamma on every frame after.
        let locked = self.session_lock.is_locked();
        let count = self.outputs.len();
        if count == 0 {
            return;
        }
        // The core's arrangement, computed once per frame and shared by
        // every output's element gathering (see `render::draw_frame`): one
        // arrange per tick, not one per output per tick. Sound because the
        // arrangement is a pure function of the core's layout inputs
        // (windows, workspaces, sizes, focus -- see `World::arrange`), and
        // none of the frame's per-output inputs (scale, output geometry,
        // lock state, pointer position) feeds into it: those are consumed
        // downstream, per output, by the element gathering the arrangement
        // is passed to. Nothing between outputs mutates the world -- the
        // frame path never calls `handle_event`/`handle_action` -- so the
        // arrangement computed here is still current at the last output.
        // Not computed at all while locked: no window and no ring is drawn
        // then, so laying the windows out would be work for frames that
        // cannot show it. `apply()` still runs the layout on every change
        // underneath, so nothing is lost by the time it unlocks.
        let arrangement = (!locked).then(|| self.world.arrange());
        #[cfg(test)]
        self.arrange_calls_for_test
            .set(self.arrange_calls_for_test.get() + usize::from(arrangement.is_some()));
        // Cleared on the first attempted draw, not unconditionally up front:
        // the tail below re-arms it on purpose (`lock_transition`,
        // `refresh_layer_zone`'s `apply`, a refused-flip retry), and a clear
        // after the loop would wipe those re-arms -- while never clearing it
        // when no output drew (no backend taken) keeps the old "a frame with
        // nothing to draw leaves the dirty flag alone" shape the suites
        // around `take_primary_backend` pin down.
        let mut attempted = false;
        // Whether something other than the cursor asked for this render:
        // taken with the first attempted draw, like `needs_render`, so a
        // re-arm by this render's own tail stays for the next one -- and
        // given back below if any output's draw failed, since that output's
        // scene change is then still not on screen.
        let mut scene = false;
        let mut every_output_drew = true;
        // Read once, here, so the element gathering, the clear colour and
        // the frame callbacks below are all answering the same question
        // about the same frame.
        let mut retry_render = false;
        let mut lock_dropped = false;
        let mut dead_layers = false;
        let time = self.start_time.elapsed();
        for index in 0..count {
            let Some((id, output)) = self.outputs.at(index) else {
                continue;
            };
            let Some(mut backend) = self.take_backend(id) else {
                // No render target for this output: skipped, leaving the
                // others to draw. Unreachable past startup -- both makers
                // (`init_named`, `add_output`) insert the target with the
                // output, and `remove_output` drops it with the output --
                // except for the suites
                // that take the primary's out by hand to prove a frame with
                // nothing to draw confirms no lock and stamps nothing (see
                // `take_primary_backend`). Skipped rather than logged: this
                // runs per frame, so a log here would flood at 60Hz.
                continue;
            };
            if !attempted {
                attempted = true;
                self.needs_render = false;
                scene = std::mem::take(&mut self.scene_dirty);
            }
            // The frame itself, drawn with whichever renderer this session is
            // carrying. See `render.rs` for why the choice of renderer is an
            // enum dispatched once per frame rather than a type parameter
            // threaded through `State`. The arrangement travels in beside
            // it: the one computed above for the whole tick, shared by
            // every output, not re-derived per output.
            let frame =
                render::draw_frame(self, &mut backend, &output, locked, arrangement.as_ref());
            self.put_backend(id, backend);
            // What `screencopy.rs` asks "has the scene moved since this
            // session's last capture?" with: only a frame whose pixels moved
            // (a render with no damage left the previous frame on screen)
            // and that something other than the cursor asked for. See
            // `State::frame_serial`.
            if frame.damaged && scene {
                self.frame_serial = self.frame_serial.wrapping_add(1);
            }
            every_output_drew &= frame.drew_a_frame;
            // A refused `--tty` flip (see `FrameOutcome::retry_render`):
            // nothing is in flight, so no `VBlank` will ever arrive to retry
            // it the way `present_skipped` retries an in-flight skip --
            // re-armed once below for the whole frame rather than per
            // output. After the flag clear below, so this request survives
            // it; bounded at the source (`present_retry.rs`), so a device
            // that keeps refusing goes quiet instead of pinning the loop.
            retry_render |= frame.retry_render;

            // The frame a pending session lock has been waiting for. Only
            // after one has actually been drawn -- never after a failed bind
            // or a failed render, which would leave whatever was on screen
            // before the lock exactly where it was -- may the client be told
            // `locked`. A lock that could not be confirmed stays pending and
            // stays *locked*; the alternative, giving up and unlocking, would
            // turn a renderer failure into an unrequested unlock (see
            // `session_lock.rs`).
            //
            // Where there is a scanout to wait for (`--tty`), confirmation
            // additionally waits for the vblank of the flip carrying the
            // blanked frame (see `SessionLock::await_vblank`): the previous,
            // possibly unlocked, frame can otherwise stay on scanout for up
            // to one more vblank after `locked` has gone out. Headless and
            // nested have no scanout, so the drawn frame *is* the shown one.
            //
            // Recorded per output, and confirmed only once every output's
            // blanked frame is in (see `SessionLock::note_blanked`): with
            // more than one screen, confirming on the first output's frame
            // would expose a live desktop on the ones that haven't blanked.
            // A second confirmation is a no-op, never an early unlock -- and
            // with one output the record completes on the first drawn frame,
            // exactly as before.
            if frame.drew_a_frame {
                if self.session_lock.awaiting_blank() && self.tty.is_some() {
                    // Per output, like the render-confirmed branch below: the
                    // wait is recorded against this output's head (each
                    // numbers its own flips), and with more than one output a
                    // screen still showing a placeholder for an admitted,
                    // undrawn lock surface records nothing -- the frame that
                    // draws it will (see `SessionLock::note_blanked` for the
                    // rule, which `note_flip_completed` applies again at the
                    // vblank). With one output nothing blocks, exactly as
                    // before.
                    let blocked = count > 1 && self.session_lock_output_blocks(&output);
                    let now = Instant::now();
                    if !blocked && self.session_lock.await_vblank(id, frame.blank_seq, now) {
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
                } else if !self.session_lock.awaiting_blank()
                    || self.session_lock.note_blanked(id, &output, count)
                {
                    // Either no lock is waiting (a no-op, as before) or this
                    // output's blanked frame just completed the set.
                    self.confirm_lock();
                }
            }

            // Presentation feedback for the frame that just went out, before
            // the frame callbacks below (Smithay's ordering: drain feedback
            // first, then tell clients to draw next). Taken if and only if
            // something was actually shown: a `--tty` flip issued, a
            // `--nested` commit handed to the host, or -- with no presenter
            // at all -- a frame drawn into the framebuffer, which *is* the
            // final image there. A rendered-but-dropped frame (busy CRTC, no
            // free host buffer, size mismatch, failed bind or draw) leaves
            // pending feedback queued for the next presented frame rather
            // than stamping a time nothing was shown at; while locked only
            // the lock surfaces are stamped (see `presentation_time.rs` for
            // the rule and the per-backend timestamp semantics).
            //
            // `vsync` is the backend, not the flip: `--tty` page flips are
            // vblank-synchronized whenever one is issued, and this arm only
            // runs when one was.
            //
            // `seq` is per-backend per the protocol's `presented` contract
            // (see `presented_frame`): the issued-flip number on `--tty`,
            // zero everywhere else -- headless has no retrace to count and
            // nested output is self-refreshing with no queryable count.
            //
            // Per output: `--nested` has exactly one output; `--headless`
            // outputs have no presenter (each drawn frame is its own shown
            // one); a `--tty` output's `blank_seq` is its own head's flip.
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
                    frame.zero_copy.as_ref(),
                );
            }

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
                //
                // OR-ed across outputs and transitioned once below: a dropped
                // lock surface may have held the keyboard, the pointer or a
                // grab (see below), and re-deriving focus per output would
                // derive it twice for one drop.
                lock_dropped |= self.lock_post_frame(&output, time);
            } else {
                // Frame callbacks for this output's own windows: a window
                // overlapping this output's geometry is told to draw, with
                // this output as the token -- which is what paces it at this
                // output's cadence rather than another's. A window with no
                // bbox yet (never mapped, nothing to overlap-test) is told
                // unconditionally, matching the old unconditional loop: a
                // client may legitimately ask for a callback before its first
                // attach, and withholding it would stall the very frame that
                // unsticks it.
                let geometry = self.space.output_geometry(&output);
                for window in self.space.elements() {
                    let on_this_output = match (geometry, self.space.element_bbox(window)) {
                        (Some(region), Some(bbox)) => region.overlaps(bbox),
                        _ => true,
                    };
                    if on_this_output {
                        window.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                            Some(output.clone())
                        });
                    }
                }
                // Override-redirect X windows are not in `self.space` either
                // (see `xwayland/unmanaged.rs`).
                #[cfg(feature = "xwayland")]
                self.x11_unmanaged_frames(&output, geometry, time);
                // Layer surfaces aren't in `self.space` either, and a bar's clock
                // stops at whatever second it first drew without this -- the same
                // frame-callback starvation the cursor surface had. Sent to every
                // mapped layer surface on this output rather than only the ones
                // that produced an element, matching both the window loop above
                // and the cursor's own reasoning: a client may legitimately ask
                // for a callback before its first attach, and withholding it
                // would stall the very frame that unsticks it.
                let dropped_dead = {
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
                // Zone re-derivation is per output (`refresh_layer_zone`
                // compares each output's own zone and no-ops where nothing
                // moved), so a sweep on any output can ask for it -- and a
                // dead surface on any output may have been holding the
                // keyboard, which is why the focus refresh rides along.
                // (`layer_destroyed` is the usual path back for both, but
                // this branch exists precisely for the teardown orders it
                // misses.)
                if dropped_dead {
                    dead_layers = true;
                }
            }
        }
        // A draw that failed (a bind or render error, logged where it
        // happened) left a scene change undrawn: keep it counted as one, so
        // the next frame that draws -- even one only the cursor asked for --
        // moves `frame_serial` and a capture that did not ask for the pointer
        // is handed the change rather than kept on the old picture. Only the
        // flag: nothing re-arms here that did not before.
        if scene && !every_output_drew {
            self.scene_dirty = true;
        }
        if retry_render {
            self.request_render();
        }
        if lock_dropped {
            // A lock surface was dropped, and it may have been the one
            // holding the keyboard, the pointer or a grab -- and what is
            // drawn just changed. Pointer focus has to be re-derived
            // explicitly and not just hit-tested: `wl_pointer.button`
            // goes to whatever the pointer last entered, and a grab
            // outlives focus changes entirely (see `session_lock.rs`).
            self.lock_transition();
        }
        if dead_layers {
            self.refresh_layer_zone();
            // One of those dead surfaces may have been holding the keyboard
            // (`layer_destroyed` is the usual path back, but this branch
            // exists precisely for the teardown orders it misses), and
            // `refresh_layer_zone` returns early when no zone moved.
            self.refresh_keyboard_focus();
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
    /// Under `--renderer gles` the target is reallocated in place, on the
    /// renderer it already has ([`Backend::resize_in_place`]): the EGL
    /// context, its shaders, every imported client texture and the EGL
    /// device all stay. pixman rebuilds its whole backend, which costs
    /// microseconds. Either way the renderer is the one the session started
    /// with (`State::renderer`), never a different one: a resize that
    /// silently changed renderers would be a session quietly different from
    /// the one that was asked for. What bounds the rate is the callers:
    /// `--tty`'s hotplug handler does not reach here unless the connector's
    /// mode actually changed, and `--nested` queues a configure's size and
    /// drains at most one per frame tick (see `Host::drain_pending_resize`)
    /// -- so this runs once per *drained* size, not once per event.
    ///
    /// A size over what the GLES context can render into is refused as
    /// above straight away. A GLES target that cannot be reallocated in
    /// place for any other reason falls back to a whole new backend, pinned
    /// to the session's EGL device like every rebuild (`State::gles_device`)
    /// -- read while the old backend is still in `backends`, which is what
    /// the pin is read off. Only if that fails too is the resize refused.
    pub fn resize_output(&mut self, width: i32, height: i32) -> bool {
        // The primary: `--nested`'s one window, and every existing
        // single-output caller. `--tty` resizes the output a hotplug
        // actually changed, through `resize_output_of`.
        let Some(id) = self.outputs.primary_id() else {
            tracing::warn!(width, height, "could not resize: there is no output yet");
            return false;
        };
        self.resize_output_of(id, width, height)
    }

    /// [`State::resize_output`] for a named output: `--tty`'s per-connector
    /// hotplug path (a monitor re-probing to a new mode), where the output
    /// that changed need not be the first. Everything the primary-only
    /// version did, scoped to `id`, plus one step only more than one output
    /// needs: the outputs are repacked side by side afterwards
    /// (`rescale_outputs` at the unchanged scale), because a screen that grew
    /// or shrank moves every screen to its right.
    pub(super) fn resize_output_of(&mut self, id: OutputId, width: i32, height: i32) -> bool {
        // Both halves in one read, so the `OutputChanged` below names the id
        // of the output that was actually resized without a second lookup
        // that could come back empty.
        let Some(output) = self.outputs.get(id).cloned() else {
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
            .backends
            .get(&id)
            .is_some_and(super::render::Backend::is_scanout)
        {
            if let Some(backend) = self.backends.get_mut(&id) {
                backend.note_resized(width, height);
            }
        } else {
            let in_place = match self.backends.get_mut(&id) {
                Some(backend) => backend.resize_in_place(&output, width, height),
                // No target to resize (only a test's backend-less output):
                // build one, as before.
                None => InPlace::Unsupported,
            };
            let resized = match in_place {
                InPlace::Resized => {
                    // debug!, per resize: the line that counts the sizes a
                    // `--nested` drag actually applied, and says none of
                    // them rebuilt the renderer.
                    tracing::debug!(width, height, "resized the render target in place");
                    Ok(())
                }
                // Refused outright, without the rebuild below: the limit is
                // the device's, and the rebuild is pinned to the same
                // device, so it would pay a new EGL display, context and
                // shader set only to fail the same way. The refusal below is
                // this path's one WARN.
                InPlace::TooLarge((max_width, max_height)) => Err(format!(
                    "{width}x{height} is larger than the GPU can render into \
                     ({max_width}x{max_height})"
                )
                .into()),
                InPlace::Unsupported | InPlace::Failed(_) => {
                    if let InPlace::Failed(error) = in_place {
                        // warn!: a GPU that will not allocate a target this
                        // size is news, and the rebuild below is about to
                        // pay a whole EGL context to try again.
                        tracing::warn!(
                            %error,
                            width,
                            height,
                            "could not resize the render target in place; rebuilding it"
                        );
                    }
                    // Pinned to the session's GLES device
                    // (`State::gles_device`), read off the backend that is
                    // still in `backends`: a rebuild that could not build
                    // there fails into the arm below rather than moving to
                    // another device, whose driver may refuse the dma-buf
                    // layouts already advertised to clients.
                    Backend::new(
                        &output,
                        width,
                        height,
                        self.renderer,
                        ScanoutHandoff::default(),
                        self.gles_device(),
                    )
                    .map(|backend| {
                        self.backends.insert(id, backend);
                    })
                }
            };
            if let Err(error) = resized {
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
        // A resized screen moves every screen to its right: repack them side
        // by side, the fold startup builds them with, before anything below
        // reads a geometry or announces one -- so output-management clients
        // hear the new mode and the moved positions in one batch. One output
        // has nothing to repack (it sits at the origin), and its wire traffic
        // stays exactly what the resize alone sends.
        self.repack_outputs();
        // The logical rectangle the core and the `Space` both work in -- see
        // `output_scale.rs`'s `logical_size`, and `init`'s comment on why the
        // core must never be handed the physical size. At the output's own
        // origin: the primary's is `(0, 0)`, which is what this always filed,
        // and any other output's is wherever it sits.
        let (origin, logical) = self
            .space
            .output_geometry(&output)
            .map(|geometry| {
                (
                    (geometry.loc.x, geometry.loc.y),
                    (geometry.size.w, geometry.size.h),
                )
            })
            .unwrap_or(((0, 0), (width, height)));
        // A lock surface's configured size is an *exact* requirement -- the
        // next buffer that doesn't match it is a `dimensions_mismatch`
        // protocol error, i.e. a killed lock client on a locked session -- so
        // a resized output has to reconfigure its own surfaces. A no-op when
        // the session isn't locked (there are none). Logical, not physical: a
        // lock surface's configure size is in surface-local logical
        // coordinates. Scoped to the output that moved: another output's
        // surfaces keep their own size.
        self.resize_lock_surfaces(&output, logical);
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
        // A floating window being dragged was measured against the old size
        // (see `floating/grab.rs`); the drag ends here.
        self.end_floating_grab();
        self.world.handle_event(CoreEvent::OutputChanged {
            id,
            area: Rect::new(origin.0, origin.1, logical.0, logical.1),
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

    /// Re-applies a new output scale to every output, after
    /// [`State::output_scale`](super::State::output_scale) has already been
    /// updated to it.
    ///
    /// Each output keeps its physical mode -- only the advertised scale
    /// moves, through the same `set_mode` startup uses, so bound `wl_output`
    /// clients hear the new integer -- and the outputs are recompacted
    /// order-preservingly onto the new logical widths (each sits immediately
    /// right of the previous one's new right edge, exactly the fold
    /// `add_output` builds fresh sessions with), so a rescaled session ends
    /// up laid out like a session started at the new scale rather than
    /// preserving stale positions into a gap or an overlap. The new logical
    /// geometry (see `output_scale.rs`'s `logical_size`) is filed with the
    /// core per output, position included (unlike `resize_output`'s
    /// origin-only rectangle, which is correct only for its single-output
    /// backends). Lock surfaces are reconfigured to their output's new
    /// logical size and layer surfaces re-arranged against it, the same two
    /// steps a resize runs; output-management heads and capture constraints
    /// are refreshed from the same sites. The caller runs `apply()` after,
    /// which pushes the re-derived arrangement onto the windows and renders.
    ///
    /// A move crosses two stores that must agree: the Space-side location
    /// `output_geometry` (and with it the input clamp, the render elements
    /// and the core areas) reads, rewritten by re-mapping the already-mapped
    /// output, and the Output-side location the wire (`wl_output.geometry`,
    /// `xdg_output.logical_position`, the output-management heads)
    /// advertises, re-sent through `set_mode`'s location. Passing a location
    /// `set_mode` never touches the Space side, and mapping never announces
    /// anything -- either half alone leaves the two disagreeing.
    ///
    /// Deliberately *not* rebuilding any render target: the framebuffer is
    /// physical pixels, and a pure scale change leaves every output's
    /// physical mode exactly the size it was (stated as a `debug_assert`
    /// below). The damage tracker needs no reset either: it reads the
    /// output's mode live (`OutputModeSource::Auto`) and evaluates every
    /// element's geometry at the current scale, so each moved element
    /// damages both its old and its new region on the first post-rescale
    /// frame by construction.
    pub(super) fn rescale_outputs(&mut self, scale: f64) {
        // Walked by index with cloned outputs, like the render loop: the
        // steps below take `&mut State`, which no borrow of `self.outputs`
        // can outlive. An `Output` clone is an `Arc` bump, no allocation
        // beyond the one small `Vec` this cold path keeps.
        let count = self.outputs.len();
        let mut moved: Vec<(OutputId, Output, Rect)> = Vec::new();
        // The running left edge, in the new logical pixels. Saturating like
        // `add_output`, for the same config-scale overflow rationale.
        let mut x = 0i32;
        for index in 0..count {
            let Some((id, output)) = self.outputs.at(index) else {
                continue;
            };
            let Some(mode) = output.current_mode() else {
                // Unreachable: `init_named` sets a mode before any caller
                // exists, and nothing since removes one. Logged, not
                // skipped silently: a `false`-shaped quiet skip is what
                // `resize_output` treats as seriously as a crash.
                tracing::warn!("could not rescale: the output has no mode yet");
                continue;
            };
            let position = Point::<i32, Logical>::from((x, 0));
            // `None` where the output already sits there, so nothing
            // re-announces an identical geometry: a single-output session
            // recompacts onto its own origin, and its wire traffic stays
            // exactly what the scale change alone sends.
            let location = self
                .space
                .output_geometry(&output)
                .map(|geometry| geometry.loc != position)
                .unwrap_or(true)
                .then_some(position);
            self.space.map_output(&output, position);
            set_mode(&output, mode.size.w, mode.size.h, location, scale);
            let Some(geometry) = self.space.output_geometry(&output) else {
                // Unreachable even beyond the mode arm above: the output was
                // just (re-)mapped into this space. Same loud-skip rule.
                tracing::warn!("could not rescale: the output has no geometry to file");
                continue;
            };
            debug_assert!(
                self.backends
                    .get(&id)
                    .is_none_or(|backend| backend.size() == (mode.size.w, mode.size.h)),
                "a pure scale change must leave the physical render target alone"
            );
            x = x.saturating_add(geometry.size.w);
            moved.push((
                id,
                output,
                Rect::new(
                    geometry.loc.x,
                    geometry.loc.y,
                    geometry.size.w,
                    geometry.size.h,
                ),
            ));
        }
        if !moved.is_empty() {
            // As on a resize: a drag measured against the old geometry ends.
            self.end_floating_grab();
        }
        for (id, output, area) in &moved {
            // A lock surface's configured size is an *exact* requirement
            // (see `resize_output`); a rescaled output reconfigures its own
            // surfaces the same way. A no-op while unlocked.
            self.resize_lock_surfaces(output, (area.w, area.h));
            // Layer surfaces are anchored to the output's edges, so every
            // one of them has moved -- same step as the resize path.
            layer_map_for_output(output).arrange();
            self.world.handle_event(CoreEvent::OutputChanged {
                id: *id,
                area: *area,
            });
        }
        // The scale changed, so every output-management client is a scale, a
        // mode and a `done` behind; captures re-read sizes that did not move
        // (a no-op that keeps this path from drifting from the resize one);
        // and the core's usable areas are re-reported now that every
        // `OutputChanged` above has landed.
        self.refresh_output_heads();
        self.refresh_capture_constraints();
        self.refresh_layer_zone();
        self.settle_floating_grab();
    }

    /// Takes output `id` away while the session runs -- a `--tty` connector
    /// that was unplugged -- and everything that output was part of. Answers
    /// whether it did.
    ///
    /// Refuses to remove the last output: with none left the core would park
    /// every window as unplaced, and nothing could render, capture or take
    /// the pointer. `--tty` never asks for that (it holds the last frame on
    /// the last screen instead -- see `tty/hotplug.rs`), and the refusal
    /// makes it impossible rather than merely avoided.
    ///
    /// In order, each step before the output leaves `State::outputs` so no
    /// frame, capture or event in between can resolve it:
    ///
    /// - layer surfaces on it are unmapped (which sends their
    ///   `wl_surface.leave`, popups included) and sent `closed` (the
    ///   protocol's answer for an output going away);
    /// - capture sessions on it are stopped, and a parked frame failed;
    /// - its gamma control is failed and its size forgotten;
    /// - its `ext-workspace-v1` group is removed from every manager;
    /// - every foreign-toplevel handle announced on it is told
    ///   `output_leave` (the matching `output_enter` for the adopting output
    ///   comes from the final `apply()`);
    /// - on the scanout tier, a surface its feedback was steering is
    ///   reverted to the default tranche;
    ///
    /// then the output itself goes: out of the `Space`, whose immediate
    /// refresh sends each window's `wl_surface.leave` *before* the global
    /// is withdrawn (a leave naming an output the client has already seen
    /// removed is unresolvable to it), its render target dropped, its
    /// `wl_output` global
    /// retired (withdrawn now, destroyed later -- see [`retire_global`]), and
    /// the core evicted (`evict_output`, the `OutputRemoved` event's
    /// reporting half), which hands its workspaces and
    /// windows to the focused output. What left is filed keyed by this
    /// monitor's identity, so a later add under a matching identity brings
    /// the still-open ones back (see `reconnect.rs`). The remaining outputs are repacked side
    /// by side, the session-lock wait forgets it (and confirms if it was the
    /// only screen still owing a blank), the pointer is brought back inside
    /// the desktop, and one refresh of heads, capture constraints, layer
    /// zones and keyboard focus plus an `apply()` tells everyone the rest.
    /// The `wlr-output-management` head is retired by that refresh.
    ///
    /// Cold: a hotplug path. A few small `Vec`s.
    pub(super) fn remove_output(&mut self, id: OutputId) -> bool {
        if self.outputs.len() <= 1 {
            tracing::warn!(
                output = id.0,
                "refusing to remove the last output; a session always keeps one"
            );
            return false;
        }
        let Some(output) = self.outputs.get(id).cloned() else {
            return false;
        };
        let layers: Vec<smithay::desktop::LayerSurface> =
            layer_map_for_output(&output).layers().cloned().collect();
        {
            let mut map = layer_map_for_output(&output);
            for layer in &layers {
                map.unmap_layer(layer);
            }
        }
        for layer in &layers {
            if self.clicked_layer.as_ref() == Some(layer) {
                self.clicked_layer = None;
            }
            self.mapped_layers.remove(layer.wl_surface());
            layer.layer_surface().send_close();
        }
        self.stop_captures_on(id);
        self.gamma_control.forget_output(id);
        self.retire_workspace_group(id);
        self.leave_removed_output(id);
        #[cfg(feature = "gpu-scanout")]
        {
            self.steer_scanout_feedback(id, false, Instant::now);
            self.scanout_feedback.forget(id);
        }
        self.end_floating_grab();

        self.space.unmap_output(&output);
        // `unmap_output` sends nothing: the `wl_surface.leave` for every
        // window (and its popups) on this output goes out at the `Space`'s
        // next refresh, which would otherwise be the render tail -- *after*
        // the `global_remove` below, so a client would be told its surface
        // left an output it no longer knows (foot logs "unmapped from
        // unknown output"). Refreshed here, so the leaves are queued before
        // the global is withdrawn. Layer surfaces were already told, by
        // `unmap_layer` above. Windows are not placed on the remaining
        // outputs until the `apply()` at the end, so this sends leaves only;
        // the matching enters come with that layout.
        self.space.refresh();
        // Before `outputs.remove` below: the name-only fallback in
        // `take_output_identity` reads the output's `wl_output` name, so
        // taking after the removal would always file under "".
        let identity = self.take_output_identity(id);
        self.backends.remove(&id);
        self.outputs.remove(id);
        retire_global(self, &output);
        // The core adopts the removed output's workspaces onto the focused
        // output and reports what left (`evict_output`, the `OutputRemoved`
        // event's reporting half). Filed here keyed by this monitor's
        // identity, so a later add under a matching identity brings the
        // still-open windows back (`State::restore_displaced`). An empty
        // snapshot files nothing: an output that held no windows has nothing
        // to come back to. A second removal under the same identity
        // overwrites the first -- the latest state wins.
        if let Some(evicted) = self.world.evict_output(id) {
            if !evicted.snapshot.workspaces.is_empty() {
                self.displaced.insert(identity, DisplacedOutput { evicted });
            }
        } else {
            // Unreachable: the output was in `State::outputs` (checked at
            // the top), and every output there is filed with the core at
            // creation. error!, not debug!: silently dropping the record
            // would strand the monitor's workspaces on the adopter with no
            // restore possible.
            tracing::error!(
                output = id.0,
                "drm: the removed output was unknown to the layout; its windows stay adopted"
            );
        }
        self.repack_outputs();
        if self.session_lock.forget_output(id, self.outputs.len()) {
            tracing::debug!(
                "session lock confirmed: the only output still owing a blank went away"
            );
            self.confirm_lock();
        }
        self.rehome_pointer();
        self.refresh_output_heads();
        self.refresh_capture_constraints();
        self.refresh_layer_zone();
        self.refresh_keyboard_focus();
        self.settle_floating_grab();
        self.apply();
        true
    }

    /// Re-tiles the outputs side by side, left to right in creation order,
    /// at the current scale -- the layout startup builds (`add_output` places
    /// each one at the previous one's right edge) -- after something changed
    /// an output's width or took one away. Returns whether any output moved.
    ///
    /// Only outputs whose left edge actually changes are touched: re-mapped
    /// in the `Space`, their new position announced on the wire
    /// (`wl_output.geometry`, `xdg_output.logical_position`) through
    /// `change_current_state` with nothing but the location, and an
    /// `OutputChanged` filed with the core at the new rectangle. Mode, scale
    /// and size are left alone -- a move is not a resize -- so lock surfaces
    /// (sized, not placed) need no reconfigure, and layer surfaces (placed
    /// relative to their output) are only re-arranged. The caller refreshes
    /// the output-management heads, capture constraints and layer zones
    /// alongside whatever else it changed, so a client hears one batch.
    ///
    /// Cold: runs on a hotplug or a mode change, never per frame. One small
    /// `Vec` of moved outputs.
    pub(super) fn repack_outputs(&mut self) -> bool {
        let count = self.outputs.len();
        let mut moved: Vec<(OutputId, Output, Rect)> = Vec::new();
        // Saturating like `add_output`, for the same config-scale overflow
        // rationale.
        let mut x = 0i32;
        for index in 0..count {
            let Some((id, output)) = self.outputs.at(index) else {
                continue;
            };
            let Some(geometry) = self.space.output_geometry(&output) else {
                continue;
            };
            let position = Point::<i32, Logical>::from((x, 0));
            x = x.saturating_add(geometry.size.w);
            if geometry.loc == position {
                continue;
            }
            self.space.map_output(&output, position);
            output.change_current_state(None, None, None, Some(position));
            moved.push((
                id,
                output,
                Rect::new(position.x, position.y, geometry.size.w, geometry.size.h),
            ));
        }
        if moved.is_empty() {
            return false;
        }
        // As on a resize: a drag measured against the old geometry ends.
        self.end_floating_grab();
        for (id, output, area) in &moved {
            layer_map_for_output(output).arrange();
            self.world.handle_event(CoreEvent::OutputChanged {
                id: *id,
                area: *area,
            });
        }
        true
    }

    /// Marks the screen dirty and makes sure the frame ticker is running to
    /// actually redraw it.
    pub fn request_render(&mut self) {
        self.scene_dirty = true;
        self.needs_render = true;
        self.ensure_ticking();
    }

    /// [`State::request_render`] for a change to the cursor alone (see
    /// `State::cursor_changed`, its one caller): the frame is redrawn the
    /// same way, but its damage does not count as a scene change
    /// ([`State::frame_serial`]).
    pub(super) fn request_cursor_render(&mut self) {
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

        let output = state.outputs.primary_id().expect("an output");
        assert_eq!(
            state.backends.get(&output).expect("a backend").size(),
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

    /// A GLES session's device is fixed at its first build: a resize and an
    /// added output both rebuild on it. What makes this more than tidiness is
    /// that the dma-buf feedback is built once, from that device's driver, and
    /// never re-sent (see `render::gles::GlesDevice`). Under pixman there is
    /// no GLES device at all, and the test says so and stops; the
    /// `SCOOT_TEST_RENDERER=gles` run is the one that pins it. Which device a
    /// pinned build lands on when it is *not* the preferred one is pinned in
    /// `render/gles.rs`'s own tests, on real devices.
    #[test]
    fn a_gles_session_rebuilds_on_the_device_it_started_on() {
        const CANVAS: i32 = 64;
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let renderer = super::super::test_support::test_renderer();
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
            renderer,
        )
        .expect("a compositor state with a wayland socket");
        init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        let Some(first) = state.gles_device() else {
            assert_eq!(renderer, crate::cli::RendererKind::Pixman);
            return;
        };
        assert!(state.resize_output(CANVAS + 16, CANVAS), "a resize");
        add_output(&mut state, "headless-2", CANVAS, CANVAS).expect("a second output");
        assert_eq!(state.backends.len(), 2);
        for backend in state.backends.values() {
            assert_eq!(
                backend.gles_device(),
                Some(first),
                "a rebuilt GLES backend moved off the session's device"
            );
        }
    }

    /// `add_output`'s failure path leaves nothing behind: no output in
    /// `State::outputs` or the core, nothing mapped in the `Space`, and the
    /// `wl_output` global `create_output` made is gone again. Driven with a
    /// size no renderer builds, under either renderer.
    #[test]
    fn a_failed_add_output_takes_back_its_global_and_mapping() {
        const CANVAS: i32 = 64;
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

        assert!(add_output(&mut state, "headless-2", UNBUILDABLE.0, UNBUILDABLE.1).is_err());
        assert_eq!(state.outputs.iter().count(), 1, "nothing registered");
        assert_eq!(state.backends.len(), 1, "no backend");
        assert_eq!(state.space.outputs().count(), 1, "nothing left mapped");

        // The global itself, which the add cannot hand back: take the same
        // two steps `add_output` takes on failure and ask the display.
        let output = create_output(&mut state, "headless-3", CANVAS, CANVAS, (CANVAS, 0));
        let global = output
            .user_data()
            .get::<OutputGlobal>()
            .expect("create_output records its global")
            .0
            .clone();
        let backend = state.display_handle.backend_handle();
        assert!(backend.global_info(global.clone()).is_ok());
        discard_output(&mut state, &output);
        // Withdrawn at once -- no client binding from now on sees it -- but
        // still there for a bind already in flight (see `retire_global`)...
        let info = backend
            .global_info(global.clone())
            .expect("the global survives its grace period");
        assert!(info.disabled, "the global is withdrawn at once");
        // ...and destroyed once the grace period has passed.
        let deadline = Instant::now() + Duration::from_secs(5);
        while backend.global_info(global.clone()).is_ok() && Instant::now() < deadline {
            event_loop
                .dispatch(Some(Duration::from_millis(10)), &mut state)
                .expect("the event loop dispatches");
        }
        assert!(
            backend.global_info(global).is_err(),
            "the wl_output global outlived its discarded output"
        );
        assert_eq!(state.space.outputs().count(), 1);
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
