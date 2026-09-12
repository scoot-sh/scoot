//! The `--tty` backend: a real DRM/KMS display, driven from a Linux
//! session (libseat) instead of a host Wayland compositor or nothing at
//! all.
//!
//! Structurally this is the same idea as `nested.rs`: another *presenter*
//! for the same pixman-rendered framebuffer `headless.rs` already draws
//! into (see `headless::render`'s call to `Tty::present`, right alongside
//! its call to `nested::Host::present`), just a different transport --
//! dumb-buffer scanout via DRM instead of `wl_shm` buffers attached to a
//! host surface. Session (libseat) + DRM device/surface + calloop wiring
//! live here; the two dumb buffers themselves live in `buffers.rs`, same
//! split as `nested.rs`/`nested/buffers.rs`.
//!
//! `--tty` is the root compositor on the machine: unlike `--nested`, there
//! is no host compositor's `WAYLAND_DISPLAY` to race against, so `init`
//! has none of `nested::init`'s ordering constraints against
//! `compositor::run`'s later `set_var`.
//!
//! Out of scope for this backend (see the commit introducing it for why):
//! cursor rendering, DRM hotplug, multi-GPU, multi-output, DPMS, output
//! scale, key repeat.

mod buffers;

use std::error::Error;

use flexwm_ipc::PointerButton;
use smithay::backend::drm::{
    DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, PlaneConfig, PlaneState,
};
use smithay::backend::input::{
    Axis, ButtonState, InputEvent, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    PointerMotionEvent,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::primary_gpu;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::drm::control::{
    Device as ControlDevice, Mode, ModeTypeFlags, connector, crtc,
};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::{Buffer as BufferSpace, DeviceFd, Physical, Rectangle, Size, Transform};

use self::buffers::BufferPool;
use super::State;
use super::keybindings::Keybindings;

/// The DRM/KMS presenter: session, device, surface, and the dumb-buffer
/// pool frames get copied into. See the module doc for how this fits next
/// to `nested::Host`.
pub struct Tty {
    /// `LibSeatSession` only holds a `Weak` reference to the underlying
    /// libseat connection -- the one strong reference is owned by its
    /// `LibSeatSessionNotifier`, registered with the event loop in `init`
    /// and never referenced again by name after that. That's what actually
    /// keeps the session alive for the process's lifetime; this field
    /// merely lets `change_vt` reach it. If a future change ever drops
    /// that notifier (e.g. deregistering it from the loop) before `Tty`
    /// itself is dropped, every call through this field starts failing
    /// with `SessionLost` -- silently from `change_vt`'s point of view,
    /// since it only logs a warning on error (see its doc).
    session: LibSeatSession,
    drm: DrmDevice,
    surface: smithay::backend::drm::DrmSurface,
    buffers: BufferPool,
    /// Kept only to `suspend()`/`resume()` in step with session pause/
    /// activate -- `LibinputInputBackend` owns its own clone of the same
    /// underlying context (see this module's `init`) to actually dispatch
    /// events from; cloning a `Libinput` context is a refcount bump on the
    /// same C object, not an independent copy, so calling `suspend`/
    /// `resume` here reaches the exact context the backend reads from.
    libinput: Libinput,
    width: i32,
    height: i32,
    /// Whether this process currently holds DRM master -- `false` while the
    /// session is paused (VT-switched away), and also `false` if a
    /// subsequent reactivation attempt's `drm.activate` itself failed (see
    /// `reactivate`) -- either way, the DRM device isn't ours to flip. Gates
    /// `present()` so nothing tries to flip a device we don't hold -- see
    /// this module's doc on pitfall #2 in the commit that introduced it.
    ///
    /// This is *not* the same question as "is the session active" --
    /// `reactivate()` can leave this `false` (a failed `drm.activate`) in a
    /// state where the session itself is already active again (libseat's
    /// `ActivateSession` already fired; only DRM master reacquisition
    /// failed). See [`session_paused`](Self::session_paused) for that
    /// question -- confusing the two turns the "dead DRM device, working
    /// keyboard, retry the VT switch" recovery path `reactivate`'s own doc
    /// describes into a silent no-op, since a VT switch is a session-level
    /// operation that doesn't depend on holding DRM master.
    active: bool,
    /// Whether libseat considers this session paused (VT-switched away) --
    /// `true` for the duration between a `PauseSession` and the next
    /// `ActivateSession`, regardless of whether that `ActivateSession`'s own
    /// `reactivate()` call manages to reacquire DRM master (see
    /// [`active`](Self::active)'s doc for why those are different
    /// questions). Gates `change_vt`: `libseat`'s `VT_ACTIVATE` is a
    /// session-level request that only the currently active session may
    /// make, so issuing it while this is `true` is a call libseat can only
    /// refuse (`EPERM`) -- see `change_vt`'s doc.
    ///
    /// `init` asserts this `false` rather than observing an event to set it
    /// -- the same shape as the mistake that made `active` wrong, so worth
    /// justifying here rather than trusting it by inspection: `init`'s own
    /// `session.open(...)` call (above) fails if the session isn't already
    /// active (libseat refuses `open_device` for an inactive client), and
    /// `init` returns that error before this struct is ever constructed --
    /// so reaching this initializer at all already proves the session is
    /// active.
    session_paused: bool,
    /// Set right after a successful `commit`/`page_flip`, cleared on the
    /// matching `VBlank`. `present()` skips (setting `present_skipped`
    /// instead of blocking) rather than flip again while this is set --
    /// flipping while a previous flip is still pending fails with EBUSY.
    flip_pending: bool,
    /// `true` for the very first frame and again right after a session
    /// reactivation, when the CRTC's state is unknown and only a full
    /// modeset (`commit`), not a `page_flip`, is safe to issue.
    needs_modeset: bool,
    /// Which buffer slot is currently scanned out (or mid-flip to), so the
    /// next flip knows which slot becomes free once *this* flip's `VBlank`
    /// confirms the previous one is off screen.
    showing: Option<usize>,
    /// The slot that will become free on the next `VBlank`, i.e. whatever
    /// `showing` held immediately before the in-flight flip was issued.
    pending_free: Option<usize>,
    /// Mirrors `nested::Host`'s `present_skipped`: set when `present()` had
    /// a frame ready but couldn't flip (a flip already in flight, or no
    /// buffer slot free). Checked on the next `VBlank` so a skipped frame
    /// doesn't leave the screen stale until some unrelated redraw happens
    /// to trigger another one.
    present_skipped: bool,
}

/// Sets up the session, DRM device and surface, and libinput, and extends
/// the keybinding table with `--tty`-only VT-switch bindings. Returns the
/// chosen connector's preferred mode size, which the caller
/// (`compositor::run`) uses in place of `--width`/`--height` when
/// initializing `headless::init`'s render target -- under `--tty` the mode
/// picks the size, there being no host to negotiate one with the way
/// `--nested` does.
pub fn init(
    loop_handle: LoopHandle<'static, State>,
    state: &mut State,
) -> Result<(i32, i32), Box<dyn Error>> {
    let (mut session, notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();

    let gpu_path = primary_gpu(&seat_name)?.ok_or("no primary GPU found on this seat")?;
    // Session::open, never a bare `std::fs::File::open` -- that would
    // compile and even run, right up until the modeset call, which then
    // fails with EACCES with no obvious link back to "forgot the session".
    let fd = session.open(&gpu_path, OFlags::RDWR | OFlags::CLOEXEC)?;
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (mut drm, drm_notifier) = DrmDevice::new(drm_fd.clone(), true)?;

    let (connector, mode) =
        find_connector_and_mode(&drm).ok_or("no connected connector with a usable mode")?;
    let surface = create_surface(&mut drm, connector, mode)
        .ok_or("no crtc on this device is usable with the chosen connector")?;

    let (mode_width, mode_height) = mode.size();
    let (width, height) = (mode_width as i32, mode_height as i32);

    let buffers = BufferPool::new(&drm_fd, width, height)?;

    let interface = LibinputSessionInterface::from(session.clone());
    let mut libinput_context = Libinput::new_with_udev(interface);
    libinput_context
        .udev_assign_seat(&seat_name)
        .map_err(|()| "could not assign the seat to libinput")?;
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    state.tty = Some(Tty {
        session,
        drm,
        surface,
        buffers,
        libinput: libinput_context,
        width,
        height,
        active: true,
        session_paused: false,
        flip_pending: false,
        needs_modeset: true,
        showing: None,
        pending_free: None,
        present_skipped: false,
    });

    loop_handle
        .insert_source(notifier, session_event)
        .map_err(|error| format!("could not register the session notifier: {error}"))?;
    loop_handle
        .insert_source(drm_notifier, drm_event)
        .map_err(|error| format!("could not register the drm device: {error}"))?;
    loop_handle
        .insert_source(libinput_backend, libinput_event)
        .map_err(|error| format!("could not register libinput: {error}"))?;

    // `--tty`-only: Ctrl+Alt+F1..F12. Kept out of `Keybindings::default()`
    // so the headless/nested tables stay exactly what they were before this
    // backend existed -- see `Keybindings::extend`'s doc. Runs after the
    // config file's binds are already loaded into `state.keybindings`, and
    // deliberately overrides rather than defers to any of them: Ctrl+Alt+Fn
    // is the one recovery path when the display is wedged on hardware with
    // no other window manager and no easy remote access (see
    // `config.rs`'s module doc), so it must always win, not silently lose
    // to a config-file typo or a well-meaning-but-dangerous rebind.
    for (mods, keysym, replaced) in state.keybindings.extend(Keybindings::vt_switch_bindings()) {
        tracing::warn!(
            ?mods,
            keysym = keysym.raw(),
            ?replaced,
            "a config-file keybinding on this combo was overridden by --tty's \
             VT-switch binding, which must always work as the recovery path"
        );
    }

    Ok((width, height))
}

/// The first `Connected` connector with at least one mode, and that mode
/// (its `PREFERRED`-flagged one if any, else its first). One output only
/// (multi-output is out of scope for this backend), so the first match
/// wins.
fn find_connector_and_mode(drm: &DrmDevice) -> Option<(connector::Handle, Mode)> {
    let resources = drm.resource_handles().ok()?;
    for &conn in resources.connectors() {
        let Ok(info) = drm.get_connector(conn, false) else {
            continue;
        };
        if info.state() != connector::State::Connected {
            continue;
        }
        let mode = info
            .modes()
            .iter()
            .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| info.modes().first())
            .copied();
        if let Some(mode) = mode {
            return Some((conn, mode));
        }
    }
    None
}

/// Tries every CRTC on the device against `conn`/`mode` until one accepts
/// a surface -- `create_surface` itself picks a compatible encoder and
/// plane, so this is the only selection left to do here.
fn create_surface(
    drm: &mut DrmDevice,
    conn: connector::Handle,
    mode: Mode,
) -> Option<smithay::backend::drm::DrmSurface> {
    let crtcs: Vec<crtc::Handle> = drm.crtcs().to_vec();
    for crtc in crtcs {
        match drm.create_surface(crtc, mode, &[conn]) {
            Ok(surface) => return Some(surface),
            Err(error) => {
                tracing::debug!(?crtc, %error, "crtc not usable with this connector");
            }
        }
    }
    None
}

impl Tty {
    /// The buffer age `headless::render` should pass `render_output` for
    /// this frame -- see `buffers.rs`'s module doc on why it isn't always
    /// the same value, and `BufferPool::next_age`'s doc for what it means.
    /// A pure peek: pair every call with [`advance_generation`](Self::advance_generation)
    /// once the render it was used for has actually happened.
    pub fn next_buffer_age(&self) -> usize {
        self.buffers.next_age()
    }

    /// Must be called exactly once per `render_output` call this backend's
    /// [`next_buffer_age`](Self::next_buffer_age) was used for -- see
    /// `BufferPool::advance_generation`'s doc for why this can't be folded
    /// into `present` itself (it must run even when `present` isn't called
    /// at all, i.e. when nothing was damaged this frame).
    pub fn advance_generation(&mut self) {
        self.buffers.advance_generation();
    }

    /// Copies an already-rendered frame's `region` into a free dumb buffer
    /// and scans it out. `pixels` holds exactly that region's own pixels
    /// (tightly packed, `region.size.w * region.size.h * 4` bytes), not the
    /// full frame -- see `buffers.rs`'s module doc on why presenting less
    /// than the whole output is the point. `frame_size` is the *output's*
    /// total size, independent of how small `region` is, and is what's
    /// checked against this backend's fixed mode size.
    ///
    /// Does nothing if the session is paused, or a reactivation attempt
    /// failed to reacquire the DRM device (`active` is `false` in either
    /// case -- see `reactivate`), if `frame_size` doesn't match this
    /// output's fixed mode size (this backend doesn't support resizing --
    /// the mode is chosen once, at startup), or if a previous flip hasn't
    /// been confirmed by a `VBlank` yet (`flip_pending`) -- flipping again
    /// before that would fail with EBUSY. The last two set `present_skipped`
    /// so a `VBlank` (or, for the
    /// paused case, a reactivation) re-triggers a render instead of leaving
    /// the screen stale.
    pub fn present(
        &mut self,
        pixels: &[u8],
        region: Rectangle<i32, Physical>,
        frame_size: (i32, i32),
    ) {
        if !self.active {
            return;
        }
        if frame_size != (self.width, self.height) {
            return;
        }
        if self.flip_pending {
            self.present_skipped = true;
            return;
        }
        let Some((index, fb)) = self.buffers.write_region(pixels, region) else {
            // Unlike the flip_pending skip above -- an ordinary, frequent,
            // harmless throttle; exactly one slot is always free whenever a
            // flip isn't in flight -- reaching here means neither slot was
            // free even though no flip is pending, which should never
            // happen in normal operation. It means either a buffer-freeing
            // bug leaked a slot (this is the failure mode a prior review
            // flagged as running silently forever once both slots are
            // stuck busy) or `write_region` failed to map a dumb buffer
            // (see its own log line in buffers.rs). warn!, not debug!: this is a
            // bug signal, not routine throttling, so it's fine for it to
            // repeat on every subsequent present() for as long as it lasts.
            tracing::warn!(
                "drm: present skipped, no free buffer slot (both slots busy \
                 with no flip pending)"
            );
            self.present_skipped = true;
            return;
        };
        self.present_skipped = false;

        let src_size: Size<i32, BufferSpace> = frame_size.into();
        let dst_size: Size<i32, Physical> = frame_size.into();
        let plane_state = PlaneState {
            handle: self.surface.plane(),
            config: Some(PlaneConfig {
                src: Rectangle::from_size(src_size).to_f64(),
                dst: Rectangle::from_size(dst_size),
                transform: Transform::Normal,
                alpha: 1.0,
                damage_clips: None,
                fb,
                fence: None,
            }),
        };

        let result = if self.needs_modeset {
            // info!, not debug!: a modeset is rare (first frame, or right
            // after a session reactivation) and is exactly the event
            // pitfall #2's verification depends on being able to grep for
            // at the default log level -- see the commit introducing this
            // backend.
            tracing::info!("drm: modeset (full commit)");
            self.surface.commit([plane_state], true)
        } else {
            // debug!, not info!: an ordinary page flip happens on every
            // redraw (a keystroke, a cursor blink) -- once cursor
            // rendering exists this could be well over 100 times a second,
            // and logging that at info! would drown out everything else at
            // the default level for no benefit once the modeset/page-flip
            // distinction above has already been proven to work.
            tracing::debug!("drm: page flip");
            self.surface.page_flip([plane_state], true)
        };
        match result {
            Ok(()) => {
                self.needs_modeset = false;
                self.flip_pending = true;
                // Whatever was showing before this flip becomes free once
                // this flip's VBlank confirms it's off screen.
                self.pending_free = self.showing.replace(index);
            }
            Err(error) => {
                tracing::warn!(%error, "drm commit/page flip failed");
                // Undo the write above -- this slot was never actually
                // sent to the CRTC, so it must not stay marked busy.
                self.buffers.mark_free(index);
            }
        }
    }

    /// Clears `flip_pending` for a `VBlank` on this surface's own crtc
    /// (`DrmEvent::VBlank` doesn't say which surface, only which crtc --
    /// this is a single-output backend, but checking costs nothing) and
    /// frees the buffer that was showing before this flip. Returns whether
    /// a render should be re-triggered because a previous `present()` had
    /// been skipped.
    fn on_vblank(&mut self, crtc: crtc::Handle) -> bool {
        if crtc != self.surface.crtc() {
            return false;
        }
        self.flip_settled()
    }

    /// The shared tail of `on_vblank` and `drm_event`'s `DrmEvent::Error`
    /// arm: whatever flip was in flight is done -- one way (confirmed by a
    /// `VBlank`) or another (its completion is now untrackable, reported as
    /// an `Error` instead) -- so the buffer slot it was about to free
    /// (`pending_free`) is safe, and necessary, to free either way; nothing
    /// else in this module will ever free that slot on our behalf. Pulled
    /// out into one method specifically so the two call sites can't drift
    /// the way they did before (`on_vblank` freed `pending_free`, the
    /// `Error` arm didn't, and the CRTC-mismatch check in `on_vblank` has no
    /// equivalent need here -- a `DrmEvent::Error` isn't scoped to a crtc).
    /// Returns whether a render should be re-triggered because a previous
    /// `present()` had been skipped.
    fn flip_settled(&mut self) -> bool {
        self.flip_pending = false;
        if let Some(index) = self.pending_free.take() {
            self.buffers.mark_free(index);
        }
        std::mem::take(&mut self.present_skipped)
    }

    /// Re-evaluates the CRTC's state and forces a full modeset on the next
    /// `present()`, rather than a `page_flip` -- required after a VT switch
    /// back: the CRTC's state is unknown at that point (another VT may
    /// have reconfigured it), and a bare page-flip onto stale state is
    /// exactly the "switch away, switch back, screen stays black forever"
    /// failure this project's standards call out as the worst kind (silent,
    /// no error anywhere). Returns whether reactivation succeeded well
    /// enough to ask for a fresh render.
    ///
    /// Every step below is attempted regardless of whether an earlier one
    /// failed -- this used to return early the moment `drm.activate` failed,
    /// which skipped `libinput.resume()` too. That left input dead on top of
    /// DRM being dead: the user's only recovery mechanism (Ctrl+Alt+Fn, a
    /// keybinding) depends on libinput being alive, so a failed reactivation
    /// used to be unrecoverable without physical/remote access. `active`
    /// still tracks `drm.activate` specifically -- that's the one thing that
    /// gates whether `present()` may safely flip -- but a dead DRM device
    /// with a working keyboard is recoverable (the user just retries the VT
    /// switch); a dead DRM device *and* a dead keyboard is not.
    fn reactivate(&mut self) -> bool {
        let drm_active = match self.drm.activate(true) {
            Ok(()) => true,
            Err(error) => {
                tracing::error!(%error, "could not reactivate the drm device");
                false
            }
        };
        if self.libinput.resume().is_err() {
            tracing::warn!("could not resume libinput after reactivation");
        }
        // `activate(true)` already resets state on the device (and every
        // surface on it) if it had been inactive -- see `DrmDevice::
        // activate`'s doc -- so this is cheap belt-and-braces when
        // `drm_active`, but still worth attempting even when it isn't:
        // without DRM master this is a harmless no-op/read rather than
        // something that needs gating, and if the device does come back on
        // some later reactivation there's no reason for stale surface state
        // to have gone unreset in the meantime.
        if let Err(error) = self.surface.reset_state() {
            tracing::warn!(%error, "could not reset drm surface state after reactivation");
        }
        self.active = drm_active;
        self.flip_pending = false;
        self.needs_modeset = true;
        // The surface's own notion of what's scanned out is gone along with
        // its state (or, if `drm.activate` failed, was never something we
        // can trust to begin with) -- either way both buffer slots are safe
        // to reuse. Unconditional: freeing slots is always safe, never
        // harmful, regardless of what else above failed. Ages are
        // invalidated for the same reason (see `BufferPool::invalidate_ages`'s
        // doc): a slot's pre-pause content is real, but nothing here can
        // vouch for what's actually on screen right now, so the next
        // present must be a full redraw rather than trusting a stale age.
        self.buffers.mark_all_free();
        self.buffers.invalidate_ages();
        self.showing = None;
        self.pending_free = None;
        drm_active
    }
}

fn session_event(event: SessionEvent, _: &mut (), state: &mut State) {
    // Scoped so the mutable borrow of `state.tty` ends before the possible
    // `state.request_render()` call below needs `state` whole again --
    // same shape as `nested_dispatch.rs`'s `Dispatch<HostBuffer>` handler.
    let needs_render = {
        let Some(tty) = &mut state.tty else {
            return;
        };
        match event {
            SessionEvent::PauseSession => {
                tracing::info!("session paused; drm master released");
                tty.active = false;
                tty.session_paused = true;
                tty.drm.pause();
                tty.libinput.suspend();
                tty.flip_pending = false;
                false
            }
            SessionEvent::ActivateSession => {
                tracing::info!("session activated");
                // Cleared here, unconditionally, before `reactivate()` runs
                // -- not folded into its result. The session *is* active
                // again the moment this event fires, regardless of whether
                // `reactivate`'s own `drm.activate` call goes on to succeed;
                // tying this to that outcome would leave `change_vt` gated
                // shut by a DRM-only failure, exactly the "dead DRM device,
                // working keyboard, retry the VT switch" case `reactivate`'s
                // doc calls out as the one meant to stay recoverable.
                tty.session_paused = false;
                tty.reactivate()
            }
        }
    };
    if needs_render {
        state.request_render();
    }
}

fn drm_event(event: DrmEvent, _: &mut Option<DrmEventMetadata>, state: &mut State) {
    let needs_render = {
        let Some(tty) = &mut state.tty else {
            return;
        };
        match event {
            DrmEvent::VBlank(crtc) => tty.on_vblank(crtc),
            DrmEvent::Error(error) => {
                tracing::warn!(%error, "drm event error");
                // Same buffer bookkeeping as a VBlank (see `flip_settled`):
                // an error means this flip's completion is no longer
                // trackable, but whatever slot it was about to free is
                // still safe, and necessary, to free -- otherwise it leaks
                // forever (there are only 2 slots total; see
                // `Tty::present`'s own log for what happens once both are
                // stuck busy). Not forcing `needs_modeset` here: a
                // `DrmEvent::Error` is Smithay reporting a fault reading the
                // DRM event fd itself, not evidence the CRTC was
                // reconfigured behind us the way a VT switch is -- the
                // surface's cached state is no more suspect than it was a
                // moment ago, and a gratuitous modeset visibly blanks the
                // screen. If the device really is wedged, the next
                // page_flip fails synchronously and is already logged at
                // its own call site.
                tty.flip_settled()
            }
        }
    };
    if needs_render {
        state.request_render();
    }
}

/// Translates libinput events into the same `State` methods every other
/// input source (IPC, `--nested`'s host-forwarded input) already funnels
/// through -- `input.rs` stays backend-neutral; all the `--tty`-specific
/// translation (relative motion, libinput's own event shapes) lives here.
fn libinput_event(event: InputEvent<LibinputInputBackend>, _: &mut (), state: &mut State) {
    match event {
        InputEvent::DeviceAdded { device } => {
            tracing::info!(name = %device.name(), "libinput device added");
        }
        InputEvent::DeviceRemoved { device } => {
            tracing::info!(name = %device.name(), "libinput device removed");
        }
        InputEvent::Keyboard { event } => {
            state.key(event.key_code(), event.state());
        }
        InputEvent::PointerMotion { event } => {
            state.pointer_move_relative(event.delta_x(), event.delta_y());
        }
        InputEvent::PointerButton { event } => {
            if let Some(button) = linux_button(event.button_code()) {
                state.pointer_button(button, event.state() == ButtonState::Pressed);
            }
        }
        InputEvent::PointerAxis { event } => {
            let dx = event.amount(Axis::Horizontal).unwrap_or(0.0);
            let dy = event.amount(Axis::Vertical).unwrap_or(0.0);
            state.scroll(dx, dy);
        }
        _ => {}
    }
}

/// Linux input event `BTN_*` codes, matching `input.rs`'s `code()` in
/// reverse -- the same mapping `nested_dispatch.rs`'s own `linux_button`
/// uses for the host-forwarded case.
fn linux_button(code: u32) -> Option<PointerButton> {
    match code {
        0x110 => Some(PointerButton::Left),
        0x111 => Some(PointerButton::Right),
        0x112 => Some(PointerButton::Middle),
        _ => None,
    }
}

/// What `State::change_vt` actually did. Exists so the IPC `key` request
/// path (`ipc.rs`'s `Request::Key` handler, via `input.rs`'s `press`/`key`)
/// can tell a real switch-away apart from a no-op: a real hardware keybind
/// has no reply channel to warn through and doesn't need one (the user is
/// physically still at the console either way), but an agent whose only
/// input *and output* is this one IPC connection needs to learn from the
/// reply itself that it just made the compositor unreachable over IPC --
/// see the backlog item this closes (`ROADMAP.md`) and `flexwm-vision`'s
/// "IPC-first, an agent doing computer-use is a first-class client" goal.
/// Only `Requested` should ever surface as a warning; `Ignored`/`Failed`
/// mean nothing actually changed, so there's nothing new to warn about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VtSwitchOutcome {
    /// `tty.session.change_vt` was actually called and returned `Ok(())` --
    /// a real `VT_ACTIVATE` request went out. This is *not* a guarantee the
    /// switch happens: libseat's own `libseat_switch_session` doc says
    /// plainly that "a call ... does not imply that a switch will occur,"
    /// and empirically (verified on the dev VM) requesting the VT this
    /// session is *already* showing on returns `Ok(())` too, with no pause
    /// and no VT change at all -- seatd just logs "requested session is
    /// already active" and does nothing further. `Requested` therefore means
    /// "asked, and the ask wasn't rejected outright," not "confirmed
    /// switched" -- see `ipc.rs`'s `Request::Key` handler for how its
    /// `Warning` message is worded to match that uncertainty rather than
    /// overclaiming a pause that may not happen.
    Requested,
    /// No `--tty` backend, or the session is already paused -- a deliberate
    /// no-op, already logged by `change_vt` itself at its own call site.
    Ignored,
    /// `tty.session.change_vt` returned an error -- already logged by
    /// `change_vt` itself via `tracing::warn!`.
    Failed,
}

impl State {
    /// Switches the kernel virtual terminal via the session -- a Linux-
    /// session concern, deliberately not routed through `State::act`/
    /// `flexwm_core::Action`. See `keybindings::Bound::ChangeVt`'s doc for
    /// why. A no-op (with a debug log) under any backend but `--tty`, since
    /// there's no session to switch, and also while this session is paused
    /// (VT-switched away, [`Tty::session_paused`]) -- `libseat`'s
    /// `VT_ACTIVATE` is a request to switch *from* the currently active
    /// session, and a paused session has no more standing to make that
    /// request than any other backgrounded process; the kernel/`libseat`
    /// correctly refuses it with `EPERM`. Real keyboard input never reaches
    /// here while paused (`session_event`'s `PauseSession` arm suspends
    /// `libinput` first), but this project's IPC `key` request is a second,
    /// independent input path that bypasses `libinput` entirely (see
    /// `flexwm-vision`'s "IPC-first" design) and so isn't gated by that
    /// suspension -- reproduced by pausing the session (a real switch-away)
    /// and then sending the switch-back combo over IPC rather than a real
    /// keypress, which hit exactly this `EPERM` before this check existed.
    ///
    /// Gated on `session_paused`, deliberately *not* `active` (whether DRM
    /// master is held): those go false independently (see
    /// [`Tty::active`]'s doc), and a failed DRM reactivation must not block
    /// a VT-switch retry -- that's the one recovery path a dead-DRM,
    /// working-keyboard state has.
    pub fn change_vt(&mut self, vt: u32) -> VtSwitchOutcome {
        let Some(tty) = &mut self.tty else {
            tracing::debug!(vt, "change_vt requested with no tty session active");
            return VtSwitchOutcome::Ignored;
        };
        if tty.session_paused {
            // info!, not debug!: this is an explicitly requested action
            // (a real keybind or an IPC `key` request) being discarded, not
            // routine internal bookkeeping -- at the default log level it
            // must leave a trace, or a user/agent whose switch-back request
            // silently does nothing has no way to tell "ignored" apart from
            // "lost".
            tracing::info!(
                vt,
                "change_vt requested while the session is paused; ignoring \
                 rather than issuing a VT_ACTIVATE libseat can only refuse"
            );
            return VtSwitchOutcome::Ignored;
        }
        match tty.session.change_vt(vt as i32) {
            Ok(()) => VtSwitchOutcome::Requested,
            Err(error) => {
                tracing::warn!(vt, %error, "could not change vt");
                VtSwitchOutcome::Failed
            }
        }
    }
}
