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
    /// `false` while the session is paused (VT-switched away). Gates
    /// `present()` so nothing tries to flip a paused device -- see this
    /// module's doc on pitfall #2 in the commit that introduced it.
    active: bool,
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
    // so the headless/nested tables stay exactly what they were before
    // this backend existed -- see `Keybindings::extend`'s doc.
    state.keybindings.extend(Keybindings::vt_switch_bindings());

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
    /// Copies an already-rendered frame into a free dumb buffer and
    /// scans it out. Same renderer-agnostic byte-slice signature as
    /// `nested::Host::present` -- see that doc for the rationale.
    ///
    /// Does nothing if the session is paused (`active` is `false`), if the
    /// frame's dimensions don't match this output's fixed mode size (this
    /// backend doesn't support resizing -- the mode is chosen once, at
    /// startup), or if a previous flip hasn't been confirmed by a `VBlank`
    /// yet (`flip_pending`) -- flipping again before that would fail with
    /// EBUSY. The last two set `present_skipped` so a `VBlank` (or, for the
    /// paused case, a reactivation) re-triggers a render instead of leaving
    /// the screen stale.
    pub fn present(&mut self, pixels: &[u8], width: i32, height: i32) {
        if !self.active {
            return;
        }
        if (width, height) != (self.width, self.height) {
            return;
        }
        if self.flip_pending {
            self.present_skipped = true;
            return;
        }
        let Some((index, fb)) = self.buffers.write_free(pixels) else {
            self.present_skipped = true;
            return;
        };
        self.present_skipped = false;

        let src_size: Size<i32, BufferSpace> = (width, height).into();
        let dst_size: Size<i32, Physical> = (width, height).into();
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
    fn reactivate(&mut self) -> bool {
        // `activate(true)` already resets state on the device (and every
        // surface on it) if it had been inactive -- see `DrmDevice::
        // activate`'s doc -- but resetting the surface again explicitly
        // below is cheap belt-and-braces, not redundant work that matters.
        if let Err(error) = self.drm.activate(true) {
            tracing::error!(%error, "could not reactivate the drm device");
            return false;
        }
        if self.libinput.resume().is_err() {
            tracing::warn!("could not resume libinput after reactivation");
        }
        if let Err(error) = self.surface.reset_state() {
            tracing::warn!(%error, "could not reset drm surface state after reactivation");
        }
        self.active = true;
        self.flip_pending = false;
        self.needs_modeset = true;
        // The surface's own notion of what's scanned out is gone along
        // with its state; both buffer slots are safe to reuse.
        self.buffers.mark_all_free();
        self.showing = None;
        self.pending_free = None;
        true
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
                tty.drm.pause();
                tty.libinput.suspend();
                tty.flip_pending = false;
                false
            }
            SessionEvent::ActivateSession => {
                tracing::info!("session activated");
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
                tty.flip_pending = false;
                false
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

impl State {
    /// Switches the kernel virtual terminal via the session -- a Linux-
    /// session concern, deliberately not routed through `State::act`/
    /// `flexwm_core::Action`. See `keybindings::Bound::ChangeVt`'s doc for
    /// why. A no-op (with a debug log) under any backend but `--tty`, since
    /// there's no session to switch.
    pub fn change_vt(&mut self, vt: u32) {
        let Some(tty) = &mut self.tty else {
            tracing::debug!(vt, "change_vt requested with no tty session active");
            return;
        };
        if let Err(error) = tty.session.change_vt(vt as i32) {
            tracing::warn!(vt, %error, "could not change vt");
        }
    }
}
