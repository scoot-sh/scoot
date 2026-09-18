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
//! multi-GPU, multi-output, DPMS, key repeat. ("Multi-GPU" there means
//! driving more than one at once -- *choosing* between several is `gpu.rs`'s
//! job, and `init` below walks its list until a device works.)
//!
//! DRM hotplug used to be on that list and no longer is: `hotplug.rs`
//! watches udev and re-runs the connector/mode choice whenever the display
//! underneath changes, still onto one output. It stays one output -- see
//! that module's doc for what a hotplug does and deliberately does not do.

mod buffers;
mod flip_tracker;
mod gpu;
mod hotplug;
mod present_retry;

pub(crate) use self::gpu::{ExplicitGpu, resolve};

use std::error::Error;
use std::path::Path;

use flexwm_ipc::PointerButton;
use smithay::backend::drm::{
    DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmEvent, DrmEventMetadata, PlaneConfig, PlaneState,
};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, ButtonState, Event, InputEvent, KeyboardKeyEvent,
    PointerAxisEvent, PointerButtonEvent, PointerMotionEvent, ProximityState,
    TabletToolButtonEvent, TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent,
    TabletToolTipState,
};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::{Event as SessionEvent, Session};
use smithay::backend::udev::UdevBackend;
use smithay::reexports::calloop::LoopHandle;
use smithay::reexports::drm::control::{Mode, connector, crtc};
use smithay::reexports::input::Libinput;
use smithay::utils::{Buffer as BufferSpace, DeviceFd, Physical, Rectangle, Size, Transform};

use smithay::input::tablet::TabletDescriptor;
use smithay::input::tablet::tool::AxisFrame;
use smithay::reexports::input::DeviceCapability as LibinputCapability;

use self::buffers::BufferPool;
use self::flip_tracker::FlipTracker;
use self::present_retry::PresentRetries;
use super::State;
use super::keybindings::Keybindings;
use super::output_scale::logical_size;

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
    /// Which udev device this backend is driving, so a `change` event for
    /// one of the seat's *other* DRM devices can be told from one for ours.
    /// `DrmDevice::device_id`'s own value, which is the `st_rdev` of the
    /// node it was opened from -- the same number `UdevBackend` keys its
    /// device table by (it `stat`s every path `all_gpus` returns), so the
    /// two really are comparable and not merely both called "device id".
    device_id: libc::dev_t,
    /// The one connector this backend drives. Startup picks it
    /// (`gpu::find_connector_and_mode`) and a hotplug can move it
    /// (`hotplug.rs`), which is the only other write site.
    ///
    /// The `DrmSurface` has its own `pending_connectors()`, and the two
    /// agree by construction -- every write here happens right after the
    /// matching `set_connectors` succeeded. This field exists because the
    /// surface's answer is a set (Smithay supports several connectors per
    /// CRTC; this backend deliberately drives exactly one), and re-deriving
    /// "the connector" from a set would mean inventing a rule for a case
    /// that cannot arise.
    connector: connector::Handle,
    /// `--mode WxH`, exactly as the user gave it, kept so a hotplug can
    /// re-run the same choice startup made rather than silently demoting
    /// the flag to a startup-only preference. `None` means the connector's
    /// preferred mode wins, at startup and at every re-probe alike.
    requested_mode: Option<(u16, u16)>,
    /// Whether the last probe of this device found *nothing* `Connected`.
    ///
    /// Only that. Not "the session is paused" ([`session_paused`](Self::session_paused))
    /// and not "DRM master is not held" ([`active`](Self::active)) -- all
    /// three can be true or false independently, and this project has been
    /// bitten before by two fields that looked like the same question
    /// (`docs/roadmap/05b-vt-switch-eperm.md`). Written only by
    /// `Tty::reconfigure`, on every probe: set when the probe found nothing,
    /// cleared when it found something. Read only there too, to decide
    /// whether a display that came back at the mode it left on still needs a
    /// modeset -- it does, because the CRTC spent the gap driving a
    /// connector that had physically gone away.
    nothing_connected: bool,
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
    ///
    /// This is the presenter's half of the session-lock vblank wait (see
    /// `session_lock.rs`): the number identifies *which* flip is out, so a
    /// completion can be matched against the flip that actually carries the
    /// blanked frame rather than against merely "a flip finished". One field
    /// for both facts -- `is_busy()` is "a flip is in flight" -- so the two
    /// can never disagree.
    flips: FlipTracker,
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
    /// Whether the last `present()` was a refused commit/page-flip owed a
    /// timer-driven retry. Two fields rather than one shared with
    /// `present_skipped` deliberately: a skip owned by a completion event
    /// (a `VBlank` is owed) and a refusal owned by the frame timer (nothing
    /// is in flight, so no event will ever arrive) are different facts with
    /// different consumers, and sharing a flag between them would be the
    /// fact-in-two-places split this project treats as its own bug class.
    /// Set only by the commit-failure arm, taken only by the render tail
    /// (see `take_retry_render`).
    retry_armed: bool,
    /// How many consecutive flips the kernel has refused -- bounds the
    /// timer-driven retries above (see `present_retry.rs`).
    retries: PresentRetries,
}

/// Sets up the session, DRM device and surface, and libinput, and extends
/// the keybinding table with `--tty`-only VT-switch bindings. Returns the
/// chosen mode's size -- the connector's preferred mode, or the `--mode`
/// the user named -- which the caller (`compositor::run`) uses in place of
/// `--width`/`--height` when initializing `headless::init`'s render target:
/// under `--tty` the mode picks the size, there being no host to negotiate
/// one with the way `--nested` does. Also returns the connector's name
/// (`HDMI-A-1`, `Virtual-1`), which the caller gives the `wl_output` so
/// clients see the screen under the name every other compositor would use.
///
/// `gpu` is the explicitly named DRM device (`--gpu PATH`, or `[tty] gpu`
/// when the flag is absent -- see [`resolve`]): `None` (the normal case)
/// means try every device on the seat, best guess first, until one works;
/// `Some` means try exactly that one. See `gpu.rs` for the ordering and for
/// what "works" means. `mode` is `--mode WxH`, applied to whichever device
/// is chosen; see `gpu::find_connector_and_mode` for the fallback when the
/// connector has no mode of that size.
pub fn init(
    loop_handle: LoopHandle<'static, State>,
    state: &mut State,
    gpu: Option<ExplicitGpu<'_>>,
    mode: Option<(u16, u16)>,
) -> Result<(i32, i32, String), Box<dyn Error>> {
    let (mut session, notifier) = LibSeatSession::new()?;
    let seat_name = session.seat();

    // Every candidate is tried before giving up, and *why* each one failed
    // is carried into the error rather than counted -- the whole point of
    // the fallback is that the first device is sometimes the wrong one, and
    // a user staring at a black screen needs to know which devices exist
    // and what each of them said, not just that something went wrong.
    let candidates = gpu::candidates(&seat_name, gpu)?;
    let (path, device) =
        gpu::first_usable(candidates, |path| open_device(&mut session, path, mode))
            .map_err(|failures| gpu::unusable_device_error(&seat_name, gpu, &failures))?;
    let Device {
        drm,
        notifier: drm_notifier,
        surface,
        connector,
        buffers,
        width,
        height,
        name,
    } = device;
    let device_id = drm.device_id();
    // info!, not debug!: which device `--tty` ended up on is the first
    // question to ask when a screen stays black, and on hardware where the
    // automatic pick is wrong it is the only thing separating "the fallback
    // worked" from "it happened to work anyway".
    tracing::info!(path = %path.display(), connector = %name, width, height, "drm: driving this device");

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
        device_id,
        connector,
        requested_mode: mode,
        nothing_connected: false,
        buffers,
        libinput: libinput_context,
        width,
        height,
        active: true,
        session_paused: false,
        flips: FlipTracker::new(),
        needs_modeset: true,
        showing: None,
        pending_free: None,
        present_skipped: false,
        retry_armed: false,
        retries: PresentRetries::new(),
    });

    // The gamma protocol's `gamma_size` is per-CRTC hardware state, and the
    // CRTC only exists once the surface above does -- so the manager is
    // constructed with the fallback in `State::new` and corrected here, still
    // before any client can bind it (the event loop hasn't started). A query
    // failure keeps the fallback; see `Tty::gamma_size`.
    if let Some(tty) = state.tty.as_ref() {
        let size = tty.gamma_size();
        state.gamma_control.set_size(size);
        tracing::info!(size, "drm: crtc gamma size");
    }

    loop_handle
        .insert_source(notifier, session_event)
        .map_err(|error| format!("could not register the session notifier: {error}"))?;
    loop_handle
        .insert_source(drm_notifier, drm_event)
        .map_err(|error| format!("could not register the drm device: {error}"))?;
    loop_handle
        .insert_source(libinput_backend, libinput_event)
        .map_err(|error| format!("could not register libinput: {error}"))?;
    watch_for_hotplug(&loop_handle, &seat_name, device_id);

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

    Ok((width, height, name))
}

/// Registers the udev monitor that delivers DRM hotplug events (see
/// `hotplug.rs`), on a best-effort basis.
///
/// Deliberately not fatal, unlike the three sources `init` registers with
/// `?` above. Those are what makes `--tty` work at all -- no session
/// notifier, no DRM events or no input and there is nothing to run. This
/// one only makes it *keep* working when the display changes underneath, so
/// a udev socket that cannot be opened has to degrade to the behaviour
/// every release before this had (mode-set once, at startup) rather than
/// refuse to start a session that would otherwise have been fine. Same
/// reasoning as `gpu::assemble`'s tolerance of a failed device listing.
fn watch_for_hotplug(loop_handle: &LoopHandle<'static, State>, seat: &str, device_id: libc::dev_t) {
    let udev = match UdevBackend::new(seat) {
        Ok(udev) => udev,
        Err(error) => {
            // warn!, not debug!: the session that follows looks completely
            // normal right up until a display is plugged in or the host
            // window is resized, and this is the only explanation for why
            // nothing happened then.
            tracing::warn!(
                %error,
                "drm: could not watch udev for display changes; this session \
                 keeps the mode it starts with"
            );
            return;
        }
    };
    // `UdevBackend` only reports `Changed` for devices in the snapshot it
    // took at construction, which is `all_gpus(seat)`'s list (verified in
    // `backend/udev.rs` at the pinned rev: the `EventType::Change` arm is
    // gated on `self.devices.contains_key(&devnum)`). `--gpu PATH` can name
    // a device that list does not contain -- a node on another seat, or one
    // udev does not tag as a GPU -- and then hotplug events for it are
    // dropped by Smithay before this module ever sees them. Saying so here
    // costs one pass over a list of at most a handful of devices, at
    // startup, and is the difference between a diagnosable limitation and
    // silence.
    if !udev.device_list().any(|(id, _)| id == device_id) {
        tracing::warn!(
            "drm: the chosen device is not in udev's list for this seat, so \
             display changes on it will not be noticed; this session keeps \
             the mode it starts with"
        );
    }
    if let Err(error) = loop_handle.insert_source(udev, hotplug::udev_event) {
        tracing::warn!(
            %error,
            "drm: could not register the udev monitor; this session keeps the \
             mode it starts with"
        );
    }
}

/// Everything `init` needs from one DRM device, once that device has
/// proven it can actually drive a display. Exists so a candidate can be
/// built and then thrown away wholesale if it turns out not to work,
/// without `init` half-assigning `state.tty`.
struct Device {
    drm: DrmDevice,
    notifier: DrmDeviceNotifier,
    surface: smithay::backend::drm::DrmSurface,
    /// The connector the surface was created against -- `Tty::connector`'s
    /// initial value. Carried out of `open_device` rather than re-derived,
    /// because that is where the choice was made.
    connector: connector::Handle,
    buffers: BufferPool,
    width: i32,
    height: i32,
    /// The connector's name (`gpu::OpenGpu::name`), for the `wl_output`.
    name: String,
}

/// Opens one candidate and builds everything on it, or says why it can't.
/// `Err` is a `gpu::Rejection` carrying a phrase completing "this device
/// ...", matching `gpu::open`'s own convention, because
/// `gpu::unusable_device_error` lists them all under the device path they
/// belong to. Every failure below is `Unusable`, never `SessionOpen`: the
/// session already handed this device over by the time any of them can
/// happen, so what failed is the device, not the seat.
///
/// `gpu::open` has already rejected -- and handed back to the session --
/// any device with no KMS resources or no connected connector, which is
/// every failure real hardware has actually reported. The three steps
/// below can still fail (a device whose KMS pipeline exists but whose
/// CRTCs all refuse the connector, or that cannot allocate dumb buffers),
/// and each one falls through to the next candidate just the same. The
/// one difference: past `DeviceFd::from`, the fd belongs to an
/// `Arc<OwnedFd>` with no way back out, so such a device is closed by
/// being dropped rather than returned to libseat -- it stays in seatd's
/// open set until the process exits. Harmless, bounded by the number of
/// GPUs on the seat, and not worth an fd-juggling workaround.
fn open_device(
    session: &mut LibSeatSession,
    path: &Path,
    requested: Option<(u16, u16)>,
) -> Result<Device, gpu::Rejection> {
    // `gpu::open` goes through `Session::open`, never a bare
    // `std::fs::File::open` -- that would compile and even run, right up
    // until the modeset call, which then fails with EACCES with no obvious
    // link back to "forgot the session".
    let gpu::OpenGpu {
        fd,
        connector,
        mode,
        name,
    } = gpu::open(session, path, requested)?;
    // Smithay logs `Unable to become drm master, assuming unprivileged mode`
    // from inside `DrmDeviceFd::new` on every run here. It is expected, and it
    // does *not* mean master wasn't acquired: master goes to whichever open
    // file is first to open the device while nothing else holds it (root has
    // nothing to do with the grant itself -- it's what lets seatd open the
    // node and manage VTs at all), and seatd's own explicit `SET_MASTER` call
    // secures it. Master is per-open-file, not per-process, and we inherit
    // that already-master file along with the fd seatd hands us. Our own
    // `SET_MASTER` is refused with `EACCES` because the kernel only permits it
    // from the process that owns the file -- `drm_master_check_perm` wants
    // `was_master && file->pid == current->tgid`, and `drm_file_update_pid`
    // deliberately never re-owns a file that was master. Smithay's resulting
    // `privileged = false` is the state this backend wants: it stops Smithay
    // issuing `SET_MASTER`/`DROP_MASTER` itself on pause/activate, which seatd
    // already does as root on every VT switch. (The identical warning also
    // fires when master genuinely isn't held -- if something else already
    // has it, seatd's own `SET_MASTER` gets `EBUSY` too, only logs it, and
    // hands the fd over anyway; that case fails loudly at modeset instead.
    // "Opened by seatd" doesn't distinguish the two; being first to open does.)
    // Measured end to end on the dev
    // VM (clients/state debugfs, a root `SET_MASTER` probe, a full VT-switch
    // cycle) -- see `vm/README.md`'s DRM-master troubleshooting entry and the
    // resolved backlog entry in
    // `docs/backlog/resolved/drm-master-unprivileged-resolved.md`.
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (mut drm, notifier) = DrmDevice::new(drm_fd.clone(), true).map_err(|error| {
        gpu::Rejection::Unusable(format!(
            "could not be initialized as a DRM device ({error})"
        ))
    })?;

    let surface = create_surface(&mut drm, connector, mode).ok_or_else(|| {
        gpu::Rejection::Unusable("has no crtc usable with the chosen connector".to_owned())
    })?;

    let (mode_width, mode_height) = mode.size();
    let (width, height) = (i32::from(mode_width), i32::from(mode_height));
    let buffers = BufferPool::new(&drm_fd, width, height).map_err(|error| {
        gpu::Rejection::Unusable(format!("could not allocate scanout buffers ({error})"))
    })?;

    Ok(Device {
        drm,
        notifier,
        surface,
        connector,
        buffers,
        width,
        height,
        name,
    })
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
    /// Entries per gamma ramp on this backend's CRTC, for
    /// `zwlr_gamma_control_v1`'s `gamma_size`.
    ///
    /// Falls back to [`FALLBACK_GAMMA_SIZE`](super::gamma_control::FALLBACK_GAMMA_SIZE)
    /// when the query fails or reports something unusable (zero -- no LUT --
    /// or absurdly large, which would turn every `set_gamma` allocation into
    /// a memory hog). A wrong-but-sane size degrades to a `failed` event on
    /// the first `set_gamma` the hardware refuses, which is the protocol's
    /// own answer for an output that doesn't support gamma tables.
    pub(super) fn gamma_size(&self) -> u32 {
        use smithay::reexports::drm::control::Device as ControlDevice;

        const MAX_SANE_GAMMA_SIZE: u32 = 4096;
        match self.drm.get_crtc(self.surface.crtc()) {
            Ok(info) => {
                let size = info.gamma_length();
                if (2..=MAX_SANE_GAMMA_SIZE).contains(&size) {
                    size
                } else {
                    tracing::warn!(
                        size,
                        "crtc reports an unusable gamma size; advertising \
                         {FALLBACK} instead",
                        FALLBACK = super::gamma_control::FALLBACK_GAMMA_SIZE,
                    );
                    super::gamma_control::FALLBACK_GAMMA_SIZE
                }
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "could not query the crtc gamma size; advertising \
                     {FALLBACK} instead",
                    FALLBACK = super::gamma_control::FALLBACK_GAMMA_SIZE,
                );
                super::gamma_control::FALLBACK_GAMMA_SIZE
            }
        }
    }

    /// Pushes one `set_gamma` ramp to the CRTC gamma LUT: three slices of
    /// [`gamma_size`](Self::gamma_size) `u16` entries (red, green, blue).
    ///
    /// There is no Smithay helper for this -- `drm`'s own `set_gamma` ioctl
    /// wrapper, on the already-open device, addressed at the surface's own
    /// CRTC (found once at init, not re-enumerated). Any failure (no DRM
    /// master after a VT switch, a driver that refuses the size) is the
    /// caller's to turn into a `failed` event; the session keeps running.
    pub(super) fn set_gamma_ramp(
        &self,
        red: &[u16],
        green: &[u16],
        blue: &[u16],
    ) -> std::io::Result<()> {
        use smithay::reexports::drm::control::Device as ControlDevice;

        self.drm.set_gamma(self.surface.crtc(), red, green, blue)
    }

    /// Whether this process currently holds DRM master -- the same question
    /// `present()` gates on. Read by `headless::render`, which skips the
    /// whole render (and the frame-callback dispatch) while this is `false`:
    /// every frame it could draw would be dropped by `present()` anyway, so
    /// drawing it only burns compositor CPU and wakes clients to paint
    /// frames nobody shows.
    pub(super) fn is_active(&self) -> bool {
        self.active
    }

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
    /// Returns the issued flip's sequence number (see `flip_tracker.rs`),
    /// or `None` when no flip went out: the session is paused, or a
    /// reactivation attempt failed to reacquire the DRM device (`active` is
    /// `false` in either case -- see `reactivate`), `frame_size` doesn't
    /// match the size of the mode currently being scanned out, a previous
    /// flip hasn't been confirmed by a `VBlank` yet (flipping again before
    /// that would fail with EBUSY), or the commit itself failed. The
    /// session-lock vblank wait matches on `Some` (see
    /// `session_lock.rs`): only an issued flip can carry the blanked frame
    /// to scanout, so a skip -- whatever its reason -- records no number.
    ///
    /// Does nothing if the session is paused, or a reactivation attempt
    /// failed to reacquire the DRM device (`active` is `false` in either
    /// case -- see `reactivate`), if `frame_size` doesn't match the size of
    /// the mode currently being scanned out, or if a previous flip hasn't
    /// been confirmed by a `VBlank` yet -- flipping again before that would
    /// fail with EBUSY.
    ///
    /// Who retries each skip, and why only two of the four arm anything:
    ///
    /// - An in-flight flip sets `present_skipped`: a *different*,
    ///   already-out frame is what will eventually confirm (a `VBlank`)
    ///   that it's safe to try again, so the flag is what makes that
    ///   confirmation retry the render instead of leaving the screen
    ///   stale. Nothing was written, so the damage history still holds
    ///   this frame's damage and the retry re-presents it.
    /// - A refused commit/page-flip (the error arm below) arms a
    ///   timer-driven retry instead (`retry_armed`, bounded by
    ///   `present_retry.rs`): nothing is in flight, so no `VBlank` will
    ///   ever arrive to consume `present_skipped`, and the pixels already
    ///   reached the slot, so the slot's age is cleared outright
    ///   (`note_write_failed`) -- otherwise the retry would read as age 1
    ///   and draw nothing on a quiet screen. See
    ///   `docs/backlog/resolved/present-skip-eats-frame-damage-done.md`.
    /// - No free buffer slot keeps setting `present_skipped` (a later
    ///   `VBlank` or `DrmEvent::Error` still converts it), but that arm is
    ///   only reachable with nothing in flight -- both slots busy and no
    ///   flip pending is the stuck-slots bug its own `warn!` names -- so
    ///   the flag is a backstop there, not the recovery.
    /// - `!active` needs nothing: the session is paused or DRM-masterless,
    ///   nothing here can retry until `reactivate()` runs, and
    ///   `reactivate()` unconditionally arms a fresh modeset and render on
    ///   its own. A `frame_size` mismatch needs nothing either: a resize
    ///   is in flight, and `State::resize_output` has already asked for a
    ///   render at the new size before this function is ever called with
    ///   the old one.
    ///
    /// The size check is a *mismatch* check, not a fixed-size one: the mode
    /// can change while the session runs (`hotplug.rs`), and the frame
    /// `headless::render` produced may have been laid out against the
    /// previous one. Dropping that frame is right -- the next render, which
    /// `State::resize_output` has already asked for, is built at the new
    /// size.
    pub fn present(
        &mut self,
        pixels: &[u8],
        region: Rectangle<i32, Physical>,
        frame_size: (i32, i32),
    ) -> Option<u64> {
        if !self.active {
            return None;
        }
        if frame_size != (self.width, self.height) {
            return None;
        }
        if self.flips.is_busy() {
            self.present_skipped = true;
            return None;
        }
        let Some((index, fb)) = self.buffers.write_region(pixels, region) else {
            // Unlike the in-flight skip above -- an ordinary, frequent,
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
            return None;
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
                let seq = self.flips.issued();
                // Whatever was showing before this flip becomes free once
                // this flip's VBlank confirms it's off screen.
                self.pending_free = self.showing.replace(index);
                self.retries.succeeded();
                Some(seq)
            }
            Err(error) => {
                tracing::warn!(%error, "drm commit/page flip failed");
                // Undo the write above -- this slot was never actually
                // sent to the CRTC, so it must not stay marked busy -- and
                // unvouch its age: the pixels reached the slot but never
                // scanout, and the retry must fully redraw rather than
                // trust the fresh `last_written` the copy stored (see
                // `BufferPool::note_write_failed`).
                self.buffers.note_write_failed(index);
                // This frame was rendered and never shown, so the screen is
                // stale by exactly the amount that was damaged -- but unlike
                // the two skips above, no completion event can retry it:
                // nothing is in flight, so no `VBlank` will ever arrive to
                // consume `present_skipped`. Arm a timer-driven retry
                // directly instead (bounded: a device that keeps refusing
                // must not pin the loop at full redraws).
                //
                // Newly reachable rather than newly wrong: `hotplug.rs`'s
                // `invalidate_scanout` discards the in-flight flip while one
                // really may still be out (see its doc for why that
                // is the safer of the two mistakes), which can put one
                // EBUSY-rejected flip between the hotplug and the first
                // frame at the new mode. Self-limiting: the retry either
                // issues (resetting the streak) or exhausts its bound and
                // goes quiet until genuine damage arrives.
                match self.retries.failed() {
                    present_retry::Retry::Arm => {
                        self.retry_armed = true;
                    }
                    present_retry::Retry::GiveUp => {
                        tracing::warn!(
                            "drm: commit/page flip keeps failing; leaving scanout \
                             as-is until new damage arrives"
                        );
                    }
                    present_retry::Retry::Quiet => {}
                }
                None
            }
        }
    }

    /// Takes whether the last `present()` was a refused flip owed a
    /// timer-driven retry (see `present_retry.rs`). Read once per frame by
    /// the render tail, which re-arms the frame timer for it -- the only
    /// consumer, since a refused flip has no completion event coming.
    pub fn take_retry_render(&mut self) -> bool {
        std::mem::take(&mut self.retry_armed)
    }

    /// Settles the in-flight flip for a `VBlank` on this surface's own crtc
    /// (`DrmEvent::VBlank` doesn't say which surface, only which crtc --
    /// this is a single-output backend, but checking costs nothing) and
    /// frees the buffer that was showing before this flip. Returns whether
    /// a render should be re-triggered because a previous `present()` had
    /// been skipped, plus the finished flip's sequence number -- `None` when
    /// the vblank names another crtc, or when nothing was in flight (a stale
    /// vblank for a flip the scanout bookkeeping has since discarded). The
    /// number is what the session-lock vblank wait matches on (see
    /// `session_lock.rs`): only the completion of the flip carrying the
    /// blanked frame confirms the lock.
    fn on_vblank(&mut self, crtc: crtc::Handle) -> (bool, Option<u64>) {
        if crtc != self.surface.crtc() {
            return (false, None);
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
    /// `present()` had been skipped, plus the finished flip's number for the
    /// session-lock wait. The `Error` arm's caller deliberately drops the
    /// number: an error means the completion is untrackable, so it must not
    /// confirm a lock -- the fallback deadline owns that wait instead (see
    /// `session_lock.rs`).
    fn flip_settled(&mut self) -> (bool, Option<u64>) {
        let completed = self.flips.settled();
        if let Some(index) = self.pending_free.take() {
            self.buffers.mark_free(index);
        }
        (std::mem::take(&mut self.present_skipped), completed)
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
        // The surface's own notion of what's scanned out is gone along with
        // its state (or, if `drm.activate` failed, was never something we
        // can trust to begin with) -- either way both buffer slots are safe
        // to reuse. Unconditional: freeing slots is always safe, never
        // harmful, regardless of what else above failed. Ages are
        // invalidated for the same reason (see `BufferPool::invalidate_ages`'s
        // doc): a slot's pre-pause content is real, but nothing here can
        // vouch for what's actually on screen right now, so the next
        // present must be a full redraw rather than trusting a stale age.
        // Shared with the hotplug path, which invalidates exactly the same
        // six things for exactly the same reason -- see
        // `hotplug.rs`'s `invalidate_scanout`.
        self.invalidate_scanout();
        drm_active
    }
}

/// Shutdown is quiet by construction: the DRM device is paused before any
/// of its pieces drop, so Smithay's restore-on-drop never fires.
///
/// Why this exists: Smithay's `AtomicDrmDevice::drop` (and its legacy twin)
/// issues one best-effort atomic commit restoring the pre-flexwm state --
/// "so that getty will be visible" -- whenever its `active` flag is still
/// set. That flag does *not* die with `Tty`: it lives behind
/// `Arc<DrmDeviceInternal>`, which `DrmDevice::new` clones into both the
/// device and the `DrmDeviceNotifier`, and `create_surface` clones once
/// more into the surface. At steady state the count is three (`Tty.drm`,
/// `Tty.surface`, and the notifier registered with the event loop in
/// `init`), so the restore runs when the *last* clone drops -- the
/// notifier's, during event-loop teardown -- which is *after* the libseat
/// notifier living in that same loop has dropped and closed the seatd
/// socket. seatd revokes DRM master on disconnect, so the restore is a race
/// against seatd's own disconnect handling that our teardown can only lose
/// sometimes: `ERROR drm_atomic ... Failed to restore previous state.
/// Error: Permission denied (os error 13)` on an otherwise clean quit
/// (strace-proven: our `close` of the seatd socket precedes the failing
/// `DRM_IOCTL_MODE_ATOMIC`; the errno is `EACCES`, not `EPERM` despite the
/// ticket's shorthand -- non-master callers fail atomic commits with
/// `EACCES`). Reordering flexwm's own locals cannot fix it: any order still
/// drops the loop's notifier clone last.
///
/// `DrmDevice::pause()` is Smithay's supported "don't touch the fd on drop"
/// (`pause`'s own doc: "This will cause the `DrmDevice` to avoid making
/// calls to the file descriptor e.g. on drop" -- just `set_active(false)`
/// plus surface bookkeeping here, since this backend runs unprivileged and
/// no master ioctls are issued). Pausing first makes the skip
/// deterministic, and the hardware outcome is byte-for-byte the already
/// field-observed failure case: the last frame stays scanned out until the
/// VT switch on session close repaints, which the dev VM's console does on
/// its own -- nothing is stuck either way. Idempotent with the
/// `PauseSession` arm's own `drm.pause()` (quit while paused was already
/// quiet), and panic-safe: no allocation, no ioctls, nothing that can fail.
impl Drop for Tty {
    fn drop(&mut self) {
        self.drm.pause();
    }
}

fn session_event(event: SessionEvent, _: &mut (), state: &mut State) {
    // Scoped so the mutable borrow of `state.tty` ends before the
    // `Reconfigured::finish` call below needs `state` whole again -- same
    // shape as `nested_dispatch.rs`'s `Dispatch<HostBuffer>` handler.
    let outcome = {
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
                // The device is gone with the session: no completion will
                // arrive for whatever was out, so its number must not linger
                // to match a lock wait recorded after it (see
                // `flip_tracker.rs`). The wait itself stays, owned by the
                // fallback deadline until the switch back re-renders.
                tty.flips.discard();
                hotplug::Reconfigured::Nothing
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
                if !tty.reactivate() {
                    hotplug::Reconfigured::Nothing
                } else {
                    // A display plugged in (or the host window resized)
                    // while this session was on another VT fired its udev
                    // event then, with no DRM master to act on it, and
                    // `Tty::reconfigure` correctly declined. Nothing else
                    // will ever deliver that event again, so the switch back
                    // has to ask the device what it says *now* rather than
                    // assume the mode it left on is still the right one.
                    //
                    // `reactivate` has already armed a full modeset and
                    // asked for a render, so the do-nothing answer this
                    // returns in the overwhelmingly common case (nothing
                    // changed while away) loses none of that -- `Nothing`
                    // here means "nothing *further*", not "no frame".
                    match tty.reconfigure() {
                        hotplug::Reconfigured::Nothing => hotplug::Reconfigured::Render,
                        further => further,
                    }
                }
            }
        }
    };
    outcome.finish(state);
}

fn drm_event(event: DrmEvent, _: &mut Option<DrmEventMetadata>, state: &mut State) {
    let (needs_render, completed) = {
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
                //
                // The finished flip's number is deliberately dropped with it:
                // an untrackable completion must not confirm a session lock
                // (see `flip_settled` and `session_lock.rs`) -- the fallback
                // deadline owns that wait.
                let (needs_render, _) = tty.flip_settled();
                (needs_render, None)
            }
        }
    };
    // The session-lock vblank wait, if any, matches on the finished flip
    // (see `State::note_flip_completed`): a no-op with no wait recorded.
    state.note_flip_completed(completed);
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
            // A tablet-tool device announces its tablet now; its tools
            // announce themselves on first proximity (see below). The
            // anvil shape at the pinned rev.
            if device.has_capability(LibinputCapability::TabletTool) {
                state.tablet_added(&TabletDescriptor::from(&device));
            }
        }
        InputEvent::DeviceRemoved { device } => {
            tracing::info!(name = %device.name(), "libinput device removed");
            if device.has_capability(LibinputCapability::TabletTool) {
                state.tablet_removed(&TabletDescriptor::from(&device));
            }
        }
        InputEvent::Keyboard { event } => {
            state.key(event.key_code(), event.state());
        }
        InputEvent::PointerMotion { event } => {
            // Both pairs: the accelerated delta moves the absolute position,
            // the pre-accel one is what relative-pointer clients read as the
            // unaccelerated vector (see `relative_pointer.rs`).
            state.pointer_move_relative(
                event.delta_x(),
                event.delta_y(),
                event.delta_x_unaccel(),
                event.delta_y_unaccel(),
            );
        }
        // Absolute pointing devices -- a USB tablet, or the "Virtual USB
        // Digitizer" that Apple's Virtualization.framework (vfkit) exposes,
        // which has ABS_X/ABS_Y and no REL_X/REL_Y at all -- never produce
        // `PointerMotion`; libinput reports their position through this event
        // instead. Without this arm the pointer sat at 0,0 forever under
        // vfkit while clicks and scrolling still arrived, all at the corner.
        // `position_transformed` maps the device's own coordinate range onto
        // the output's *logical* size, the same space `pointer_move` and the
        // relative path's clamp use.
        InputEvent::PointerMotionAbsolute { event } => {
            let (width, height) = state.output.as_ref().map(logical_size).unwrap_or((0, 0));
            let position = event.position_transformed((width, height).into());
            state.pointer_move(position.x, position.y);
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
        // Drawing-tablet tools, the `tablet.rs` half: positions map onto
        // the output's *logical* size exactly like the absolute-pointer
        // arm above, and every event then runs through the same
        // pointer/button paths -- a pen moves the cursor and clicks, it
        // does not grow a second focus system.
        InputEvent::TabletToolProximity { event } => {
            let position = tablet_position(state, &event);
            state.tablet_proximity(
                &TabletDescriptor::from(&event.device()),
                &event.tool(),
                event.state() == ProximityState::In,
                position.x,
                position.y,
                axis_frame(&event),
            );
        }
        InputEvent::TabletToolAxis { event } => {
            let position = tablet_position(state, &event);
            state.tablet_motion(&event.tool(), position.x, position.y, axis_frame(&event));
        }
        InputEvent::TabletToolTip { event } => {
            let position = tablet_position(state, &event);
            // Fully qualified: the concrete event carries an inherent
            // `tip_state` (the input crate's `TipState`) that shadows the
            // Smithay trait method in method-call syntax.
            let down = TabletToolTipEvent::tip_state(&event) == TabletToolTipState::Down;
            state.tablet_tip(&event.tool(), down, position.x, position.y);
        }
        InputEvent::TabletToolButton { event } => {
            // Fully qualified, same shadowing as the tip arm above: the
            // inherent `button_state` answers the input crate's
            // `ButtonState`, not Smithay's.
            let pressed = TabletToolButtonEvent::button_state(&event) == ButtonState::Pressed;
            state.tablet_button(&event.tool(), event.button(), pressed);
        }
        _ => {}
    }
}

/// Maps one tablet-tool event's device position onto the output's logical
/// size -- the same space `pointer_move`, the core and every surface lay
/// out in. Shared by the proximity/axis/tip arms above so the three cannot
/// disagree about where the tool is; the button arm carries no position.
fn tablet_position(
    state: &State,
    event: &impl TabletToolEvent<LibinputInputBackend>,
) -> smithay::utils::Point<f64, smithay::utils::Logical> {
    let (width, height) = state.output.as_ref().map(logical_size).unwrap_or((0, 0));
    event.position_transformed((width, height).into())
}

/// The axis changes one libinput tool event carries, as the Smithay frame
/// the tool half batches them in. Only changed axes are set -- an
/// unchanged axis is `None`, not a restated zero, which is what keeps a
/// hovering pen from pinning pressure at whatever it last touched at.
fn axis_frame(event: &impl TabletToolEvent<LibinputInputBackend>) -> AxisFrame {
    let mut frame = AxisFrame::new();
    if event.pressure_has_changed() {
        frame = frame.pressure(event.pressure());
    }
    if event.distance_has_changed() {
        frame = frame.distance(event.distance());
    }
    if event.tilt_has_changed() {
        frame = frame.tilt(event.tilt_x(), event.tilt_y());
    }
    if event.rotation_has_changed() {
        frame = frame.rotation(event.rotation());
    }
    if event.slider_has_changed() {
        frame = frame.slider(event.slider_position());
    }
    if event.wheel_has_changed() {
        frame = frame.wheel(event.wheel_delta(), event.wheel_delta_discrete());
    }
    frame
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
/// can tell these cases apart to warn appropriately: a real hardware
/// keybind has no reply channel to warn through and doesn't need one (the
/// user is physically still at the console either way), but an agent whose
/// only input *and output* is this one IPC connection needs to learn from
/// the reply itself whenever this call didn't accomplish what a normal
/// switch-back would -- see the backlog item this closes
/// (`docs/roadmap/05b-vt-switch-eperm.md`) and
/// `flexwm-vision`'s "IPC-first, an agent doing computer-use is a
/// first-class client" goal.
///
/// Originally this collapsed "no `--tty` backend at all" and "session is
/// currently paused" into one `Ignored` case -- both looked the same from
/// `change_vt`'s point of view (an early return, nothing sent to libseat),
/// but they mean very different things to an IPC caller: the first is
/// simply "this request makes no sense on this backend," while the second
/// is exactly the one-way-door scenario this item exists to fix -- an agent
/// retrying its switch-back combo while paused needs to hear that clearly,
/// not read it as indistinguishable from `Ok`. Split into `Ignored` and
/// `IgnoredPaused` so `ipc.rs` can warn on the second and stay silent on
/// the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VtSwitchOutcome {
    /// `tty.session.change_vt` was actually called and returned `Ok(())` --
    /// a real `VT_ACTIVATE` request went out for a VT *other* than the one
    /// already displayed (the same-VT no-op below is filtered out before the
    /// call, so reaching libseat at all already means this asks for a
    /// change). This is still *not* a guarantee the switch happens:
    /// libseat's own `libseat_switch_session` doc says plainly that "a call
    /// ... does not imply that a switch will occur" -- see `ipc.rs`'s
    /// `Request::Key` handler for how its `Warning` message is worded to
    /// match that uncertainty rather than overclaiming a pause that may not
    /// happen.
    Requested,
    /// The requested VT is the one the kernel is already displaying, so the
    /// `VT_ACTIVATE` would have been a no-op (libseat returns `Ok(())` for
    /// it too, verified on the dev VM -- seatd just logs "requested session
    /// is already active" and does nothing further). Filtered out before
    /// the libseat call, so unlike `Requested` there is not even a request
    /// to hedge about: `ipc.rs` answers a plain `Ok`, not a `Warning`.
    /// Distinct from plain `Ignored` because the reason differs (a verified
    /// no-op on a live session, not "no backend to ask"), and the
    /// exhaustive `ipc.rs` match must decide it deliberately rather than
    /// inherit silence meant for another case.
    IgnoredSameVt,
    /// No `--tty` backend at all -- there's no session for this request to
    /// mean anything to. A deliberate no-op, already logged by `change_vt`
    /// itself at its own call site. Nothing to warn an IPC caller about:
    /// this isn't specific to `key`/IPC, and it can't be a one-way-door
    /// symptom since there was never a door to begin with.
    Ignored,
    /// The session is currently paused, so `change_vt` deliberately didn't
    /// even ask libseat (see its own doc for why: libseat can only refuse
    /// with `EPERM`). Distinct from plain `Ignored` because this *is* the
    /// one-way-door scenario -- an IPC caller retrying its switch-back combo
    /// needs to hear "this can't work over IPC right now," not silence
    /// indistinguishable from success.
    IgnoredPaused,
    /// `tty.session.change_vt` returned an error -- already logged by
    /// `change_vt` itself via `tracing::warn!`. The request itself failed,
    /// so this isn't a "success with a side effect" the way `Requested` is.
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
            return VtSwitchOutcome::IgnoredPaused;
        }
        if same_vt_noop(tty.session_paused, vt, displayed_vt()) {
            // debug!, not info!: unlike the paused skip above, nothing the
            // caller asked for is being lost -- the kernel is already
            // showing exactly what was requested, so there is no action to
            // trace, only a syscall spared and a warning not sent.
            tracing::debug!(
                vt,
                "change_vt requested for the VT already displayed; skipping \
                 the libseat call"
            );
            return VtSwitchOutcome::IgnoredSameVt;
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

/// Which VT the kernel is currently displaying, read live from sysfs -- or
/// `None` when that cannot be answered (no sysfs here, unreadable node,
/// unparsable content). Every `None` shape falls through to the old
/// behaviour (ask libseat, report the hedged `Requested`): a same-VT
/// request warned about is cosmetic noise, but a real switch skipped would
/// be a silent failure, so uncertainty must always resolve toward asking.
///
/// Read per `change_vt` call rather than stored on `Tty` at `init`: the
/// ticket's anticipated init-time query (which VT is this session on?)
/// has no libseat answer, but the per-call question (which VT is displayed
/// *right now*?) does, straight from the kernel -- and a session property
/// that never changes needs no field with write-site semantics to audit
/// (see `docs/roadmap/05b-vt-switch-eperm.md` for what that audit costs).
/// `change_vt` runs only on an explicit VT-switch keybind or IPC `key`,
/// never on a hot path, so one small file read per call is negligible.
fn displayed_vt() -> Option<u32> {
    parse_active_vt(&std::fs::read_to_string("/sys/class/tty/tty0/active").ok()?)
}

/// Parses `/sys/class/tty/tty0/active`'s content -- `ttyN` plus a trailing
/// newline -- into the displayed VT number. Strict by design: anything that
/// is not exactly `tty` followed by a nonzero decimal number (`ttyS0` on a
/// serial console, an empty read, garbage) is `None`, which `change_vt`
/// treats as "unknown, ask libseat" rather than evidence of anything.
fn parse_active_vt(content: &str) -> Option<u32> {
    content
        .trim()
        .strip_prefix("tty")?
        .parse::<u32>()
        .ok()
        .filter(|&n| n != 0)
}

/// Whether a VT-switch request is a provable no-op: the session is active
/// (so the displayed VT is necessarily its own -- an inactive session's VT
/// is someone else's by definition) and the request names exactly that VT.
///
/// `session_paused` is an explicit parameter rather than read off `Tty` so
/// this stays a pure predicate with a unit-testable truth table. The
/// `!session_paused` half is load-bearing, not redundant with `change_vt`'s
/// own paused gate above it: while paused, an equality between the request
/// and a displayed-VT read could only be a race, and skipping the call on
/// that basis would be exactly the one-way-door silence `IgnoredPaused`
/// exists to prevent -- so a paused session never no-ops here, whatever
/// the kernel reports.
fn same_vt_noop(session_paused: bool, requested: u32, displayed: Option<u32>) -> bool {
    !session_paused && displayed == Some(requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_vt_content_parses_to_its_number() {
        // Exactly what the kernel writes: `ttyN` plus a trailing newline.
        assert_eq!(parse_active_vt("tty1\n"), Some(1));
        assert_eq!(parse_active_vt("tty12\n"), Some(12));
    }

    #[test]
    fn non_vt_consoles_are_unknown_not_zero() {
        // A serial console (`ttyS0`) is not a switchable VT at all: this
        // must be "unknown, ask libseat", never a number that could compare
        // equal to a request. The fail-first pin for the strict parse --
        // accept a bare numeric suffix and this returns `Some(0)`.
        assert_eq!(parse_active_vt("ttyS0\n"), None);
        assert_eq!(parse_active_vt("tty0\n"), None);
    }

    #[test]
    fn garbage_content_is_unknown() {
        assert_eq!(parse_active_vt(""), None);
        assert_eq!(parse_active_vt("tty\n"), None);
        assert_eq!(parse_active_vt("console\n"), None);
        assert_eq!(parse_active_vt("tty4294967297\n"), None);
        assert_eq!(parse_active_vt("tty1\ntty2\n"), None);
    }

    #[test]
    fn an_active_session_requesting_the_displayed_vt_is_a_noop() {
        // The fail-first pin for the gate -- drop either conjunct and this
        // fails.
        assert!(same_vt_noop(false, 1, Some(1)));
    }

    #[test]
    fn an_active_session_requesting_another_vt_is_real() {
        assert!(!same_vt_noop(false, 2, Some(1)));
    }

    #[test]
    fn an_unknown_displayed_vt_never_noops() {
        // Unreadable sysfs, serial console, unparsable content: uncertainty
        // resolves toward asking libseat (the hedged `Requested`), never
        // toward a skip that could strand a real switch.
        assert!(!same_vt_noop(false, 1, None));
    }

    #[test]
    fn a_paused_session_never_noops_even_when_the_numbers_match() {
        // While paused the displayed VT belongs to another session, so an
        // equality here can only be a stale read -- and skipping the call
        // on that basis would be the one-way-door silence `IgnoredPaused`
        // exists to prevent. The paused gate in `change_vt` owns this case.
        assert!(!same_vt_noop(true, 1, Some(1)));
    }
}
