//! Which DRM device `--tty` drives, and whether it can actually drive a
//! display.
//!
//! Split out of `mod.rs` because picking a device turned out to be a real
//! problem rather than one `primary_gpu()` call. Smithay's
//! [`primary_gpu`] ranks candidates (1) a PCI parent with `boot_vga=1`,
//! (2) alphabetically first among those with a DRM *render* node, (3)
//! alphabetically first. On a PC that lands on the right device; on an SoC
//! whose 3D GPU and display controller are *separate* DRM devices -- Apple
//! Silicon under Asahi Linux, where `asahi`/AGX has a render node and
//! `apple,dcp` owns the CRTCs and connectors -- rule (1) can never match
//! (no PCI, no VGA BIOS) and rule (2) picks the render-only compute GPU.
//! Loading KMS resources on that device fails with `ENOTSUP`, which is
//! exactly the failure reported from real hardware (see
//! `docs/roadmap/17-drm-device-selection.md`).
//!
//! So: [`candidates`] returns `primary_gpu()`'s pick *first* -- it is
//! right on ordinary hardware and must keep winning there -- followed by
//! every other device on the seat, and `mod.rs`'s `init` walks that list
//! until one works. [`open`] is the per-candidate gate: it opens the
//! device through the session and proves, before anything takes ownership
//! of the file descriptor, that KMS resources load and some connector is
//! connected with a usable mode.
//!
//! Connector and mode choice ([`find_all`], and [`connector_mode`] for the
//! hotplug path) lives here too, for the same reason:
//! it is the other half of "can this device drive a display", and it has to
//! run twice -- once on a borrowed fd before the device is adopted, and
//! again on the live `DrmDevice` every time the connectors change underneath
//! a running session (see `tty/hotplug.rs`). One implementation, two
//! callers, rather than a startup copy and a hotplug copy free to drift.
//!
//! Doing the startup check on a *borrowed* fd is the point of [`Probe`]. A
//! rejected candidate can then be handed straight back to libseat with
//! `Session::close`, which needs the `OwnedFd` -- once the fd goes into
//! Smithay's `DeviceFd` (an `Arc<OwnedFd>` with no way back out) that is
//! no longer possible. It also keeps a hopeless candidate from ever
//! constructing a `DrmDeviceFd`, whose own startup logging ("Unable to
//! become drm master...") would otherwise be emitted once per device and
//! read like the cause of the failure rather than noise.

use std::fmt::{self, Write as _};
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use smithay::backend::session::Session;
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::udev::{all_gpus, primary_gpu};
use smithay::reexports::drm::Device as BasicDevice;
use smithay::reexports::drm::control::{
    Device as ControlDevice, Mode, ModeTypeFlags, ResourceHandles, connector, crtc,
};
use smithay::reexports::rustix::fs::OFlags;

use crate::compositor::output_identity::{EdidIdentity, parse_edid};

/// A device that has been opened through the session and proven to have a
/// display pipeline: its KMS resources loaded and at least one of its
/// connectors is connected with a usable mode.
///
/// The connectors were read through [`Probe`], which borrows
/// *this same fd* -- the one the caller goes on to build its `DrmDevice`
/// from -- rather than opening the device a second time. That is
/// deliberate, not incidental: Smithay's `LibSeatSession` keys its device
/// table by the raw fd libseat handed out (`devices: RefCell<HashMap<RawFd,
/// libseat::Device>>`, `backend/session/libseat.rs` at the pinned rev), so
/// a second open would be a second entry and a second device held open by
/// seatd, which only `Session::close` on *that* fd can release. Borrowing
/// costs nothing in exchange: the probe is finished before `fd` moves on,
/// a `connector::Handle` is a device-global kernel object id rather than a
/// per-fd token, and `Mode` is a plain `Copy` description of a timing.
pub struct OpenGpu {
    /// Ownership passes to the caller, which means so does the
    /// responsibility to keep it (or to drop it, closing the fd). Every
    /// path in this module that does *not* return an `OpenGpu` has
    /// already handed the device back to the session.
    pub fd: OwnedFd,
    /// Every connector that can drive a display, in the kernel's own order.
    /// Never empty: a device with none is rejected in [`open`]. The first
    /// entry is exactly the one the single-output search used to return, so
    /// a one-connector machine is driven exactly as before.
    pub connected: Vec<Connected>,
}

/// One connector that can drive a display right now, and what driving it
/// would take.
#[derive(Clone, Debug)]
pub struct Connected {
    pub connector: connector::Handle,
    pub mode: Mode,
    /// The connector's conventional name -- `HDMI-A-1`, `eDP-1`, `Virtual-1`
    /// -- as every other compositor exposes it, built from the interface
    /// type and the kernel's per-type index. It becomes the `wl_output`
    /// name, so bars and shells label the screen by it; before this the
    /// output was called `headless` on every backend, this one included.
    pub name: String,
    /// The CRTCs this connector's encoders can be routed to (their
    /// `possible_crtcs` masks, resolved against the device's CRTC list), in
    /// the device's CRTC order -- what `crtcs::assign` matches over. Empty
    /// when no encoder could be read, which leaves the connector dark rather
    /// than guessing a route.
    pub crtcs: Vec<crtc::Handle>,
    /// The connector's EDID summary, where it has an EDID blob to read
    /// (`None` for a panel with no serial, a KVM hiding the blob, or a blob
    /// that refuses). Read here, beside the mode choice, so startup and the
    /// hotplug re-probe learn the same identity the same way: it becomes the
    /// output's [`OutputIdentity`](crate::compositor::output_identity::OutputIdentity),
    /// which is what tells a replugged monitor apart from a different one on
    /// the same connector. Two small ioctls per usable connector, on paths
    /// that run at startup and when a cable moves -- never per frame.
    pub edid: Option<EdidIdentity>,
}

/// An explicitly named DRM device: the path, and where the name came from.
///
/// `--gpu PATH` wins over `[tty] gpu` when both name one (an explicit flag
/// beats a file, the way `--config` beats the default path); the source is
/// also whose name the startup error uses when the named device cannot be
/// driven (see [`unusable_device_error`]) -- a user who set the config key
/// and never typed `--gpu` must not be told the device came from `--gpu`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitGpu<'a> {
    /// `--gpu PATH` on the command line.
    Flag(&'a Path),
    /// `[tty] gpu` in the config file.
    Config(&'a Path),
}

impl<'a> ExplicitGpu<'a> {
    /// The named device.
    pub fn path(self) -> &'a Path {
        match self {
            Self::Flag(path) | Self::Config(path) => path,
        }
    }

    /// Whose name the startup error uses: `--gpu PATH` for the flag,
    /// `[tty] gpu = "PATH"` for the config key. Quoted in the config
    /// form (unlike the flag form, where the path starts its own clause
    /// below) because there it sits mid-sentence, where a path with a
    /// space in it would otherwise dissolve into the prose around it.
    fn describe(self) -> String {
        match self {
            Self::Flag(path) => format!("`--gpu {}`", path.display()),
            Self::Config(path) => format!("`[tty] gpu = \"{}\"`", path.display()),
        }
    }
}

/// Picks the explicitly named device, if any: `--gpu PATH` wins over
/// `[tty] gpu` when both name one, and no source at all means the automatic
/// search picks (see [`candidates`]).
///
/// An explicitly-set-but-empty path in *either* source is a hard startup
/// error rather than something the session is asked to open or, worse,
/// silently dropped in favour of the automatic pick. An empty path can
/// never name a device, so refusing it changes no working configuration --
/// it only turns a session-layer `ENOENT` (or a silent fallback) into a
/// refusal that names the surface that actually set it.
pub fn resolve<'a>(
    flag: Option<&'a Path>,
    config: Option<&'a Path>,
) -> Result<Option<ExplicitGpu<'a>>, EmptyGpuPath> {
    match (flag, config) {
        (Some(path), _) if path.as_os_str().is_empty() => Err(EmptyGpuPath::flag()),
        (Some(path), _) => Ok(Some(ExplicitGpu::Flag(path))),
        (None, Some(path)) if path.as_os_str().is_empty() => Err(EmptyGpuPath::config()),
        (None, Some(path)) => Ok(Some(ExplicitGpu::Config(path))),
        (None, None) => Ok(None),
    }
}

/// An explicitly-set-but-empty DRM device path: `--gpu` with nothing after
/// it, or `gpu = ""` under `[tty]`. The one config-adjacent failure that
/// stops startup -- see [`resolve`].
#[derive(Debug, PartialEq, Eq)]
pub struct EmptyGpuPath {
    source: EmptySource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptySource {
    Flag,
    Config,
}

impl EmptyGpuPath {
    fn flag() -> Self {
        Self {
            source: EmptySource::Flag,
        }
    }

    fn config() -> Self {
        Self {
            source: EmptySource::Config,
        }
    }
}

impl fmt::Display for EmptyGpuPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.source {
            EmptySource::Flag => write!(
                f,
                "`--gpu` names an empty device path; pass a DRM device \
                 (e.g. `--gpu /dev/dri/card1`) or drop the flag"
            ),
            EmptySource::Config => write!(
                f,
                "`[tty] gpu` is set but empty; name a DRM device \
                 (e.g. `gpu = \"/dev/dri/card1\"`) or remove the key"
            ),
        }
    }
}

impl std::error::Error for EmptyGpuPath {}

/// Every device worth trying, best guess first.
///
/// `explicit` (a `--gpu PATH` or a `[tty] gpu`) replaces the search
/// outright: exactly that one path, no fallback, because the whole point
/// of naming a device is to be obeyed on hardware whose automatic choice is
/// wrong. Nothing here checks that the path exists -- [`open`] reports what
/// the session says about it, which is both more accurate and free of a
/// check-then-open race.
///
/// Otherwise: `primary_gpu()`'s pick first (it is the correct answer on
/// ordinary hardware, and the fallback must not demote it there), then
/// every other device on the seat in `all_gpus()`'s own sorted order.
/// `primary_gpu()` always returns one of `all_gpus()`'s entries, so
/// filtering it out of the tail is what keeps a device from being tried
/// twice.
///
/// The tail is best-effort: see [`assemble`] for why enumerating it is
/// allowed to fail without taking the primary down with it.
pub fn candidates(seat: &str, explicit: Option<ExplicitGpu<'_>>) -> io::Result<Vec<PathBuf>> {
    if let Some(named) = explicit {
        return Ok(vec![named.path().to_owned()]);
    }
    assemble(primary_gpu(seat)?, all_gpus(seat))
}

/// Combines the two udev queries [`candidates`] makes, split out so the
/// interesting case -- one of them failing -- can be tested without a seat.
///
/// `primary_gpu()` failing is still fatal (it is `?`-ed by the caller,
/// exactly as before this fallback existed). `all_gpus()` failing is not:
/// that call only produces the *tail*, and a transient udev error while
/// enumerating it must not take down a machine whose primary device was
/// found and would have worked -- that would make this fallback a
/// regression on the hardware it is supposed to leave alone. With no
/// primary to fall back *to*, though, there is nothing left to try, and
/// the enumeration error is the honest thing to report rather than
/// [`unusable_device_error`]'s "is a GPU present?", which would blame the
/// hardware for a failure that was udev's.
fn assemble(primary: Option<PathBuf>, rest: io::Result<Vec<PathBuf>>) -> io::Result<Vec<PathBuf>> {
    match (primary, rest) {
        (primary, Ok(rest)) => Ok(order(primary, rest)),
        (Some(primary), Err(error)) => {
            // warn!, not debug!: the run that follows looks completely
            // normal on hardware where the primary is right, and is missing
            // every fallback candidate on hardware where it is not.
            tracing::warn!(
                %error,
                path = %primary.display(),
                "drm: could not list the seat's other devices; trying only the primary"
            );
            Ok(vec![primary])
        }
        (None, Err(error)) => Err(error),
    }
}

/// The ordering half of [`candidates`], split out so it can be tested
/// without a seat. See that function's doc for the rules.
fn order(primary: Option<PathBuf>, rest: Vec<PathBuf>) -> Vec<PathBuf> {
    let Some(primary) = primary else {
        return rest;
    };
    let mut ordered = Vec::with_capacity(rest.len() + 1);
    ordered.extend(rest.into_iter().filter(|path| *path != primary));
    ordered.insert(0, primary);
    ordered
}

/// Why one candidate was rejected, and -- the part that matters beyond
/// printing it -- at which stage.
///
/// The text is a lowercase phrase completing "this device ...", which is
/// how [`unusable_device_error`] lists it. The variant is what separates
/// "the session would not hand this device over" from "the device came
/// back and cannot drive a display": only the second is something naming
/// a different device could route around, and telling them apart is what
/// keeps the final error from recommending `--gpu` for a problem that has
/// nothing to do with which device was picked.
#[derive(Debug, PartialEq, Eq)]
pub enum Rejection {
    /// `Session::open` refused, so nothing here is a statement about the
    /// device itself: a seat that will not take this process refuses every
    /// device on it identically (seatd's "seat is VT-bound and has an
    /// active client" is exactly that shape).
    SessionOpen(String),
    /// The device opened, and then could not drive a display: no KMS
    /// pipeline, nothing connected with a usable mode, or one of the
    /// later setup steps `tty/mod.rs` runs on it failed. This is the
    /// failure the fallback -- and `--gpu` -- exist for.
    Unusable(String),
}

impl Rejection {
    fn is_session_open(&self) -> bool {
        matches!(self, Self::SessionOpen(_))
    }

    /// The phrase, without the stage. Borrowed rather than rendered
    /// through `Display` so the one caller that needs it mid-sentence
    /// doesn't allocate a second copy of a string it already has.
    fn reason(&self) -> &str {
        match self {
            Self::SessionOpen(reason) | Self::Unusable(reason) => reason,
        }
    }
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

/// Walks [`candidates`] in order and returns the first one `open` accepts,
/// or -- if none is accepted -- every candidate paired with why it was
/// rejected, in the order they were tried, ready for
/// [`unusable_device_error`].
///
/// `open` is a parameter rather than a direct call to [`open`] because
/// "usable" is not settled until `mod.rs` has built a `DrmDevice`, a
/// surface and its buffers on the device; this is the only part of that
/// sequence that can be exercised without a seat, and it is the part the
/// whole fallback rests on. Short-circuits: a candidate after the first
/// accepted one is never opened, which is what keeps ordinary single-GPU
/// hardware doing exactly one `Session::open` as it always has.
pub fn first_usable<T>(
    candidates: Vec<PathBuf>,
    mut open: impl FnMut(&Path) -> Result<T, Rejection>,
) -> Result<(PathBuf, T), Vec<(PathBuf, Rejection)>> {
    let mut failures = Vec::new();
    for path in candidates {
        match open(&path) {
            Ok(device) => return Ok((path, device)),
            Err(reason) => {
                // warn!, not debug!: on hardware where the first pick is
                // wrong this is the only trace of what was rejected and
                // why, and the run that follows it looks completely normal.
                tracing::warn!(path = %path.display(), reason = %reason, "drm: device unusable");
                failures.push((path, reason));
            }
        }
    }
    Err(failures)
}

/// Opens one candidate through the session and checks it can drive a
/// display. `Err` is a [`Rejection`] -- which stage failed, and a
/// lowercase phrase completing "this device ..." for
/// [`unusable_device_error`] to list; a rejected device has already been
/// closed and returned to the session by the time it is returned.
pub fn open(
    session: &mut LibSeatSession,
    path: &Path,
    requested: Option<(u16, u16)>,
) -> Result<OpenGpu, Rejection> {
    // No `OFlags::CLOEXEC`: the pinned Smithay's `LibSeatSession::open`
    // takes `_flags` and never reads it (`backend/session/libseat.rs` at the
    // pinned rev forwards only the path to libseat), so requesting the flag
    // here was dead code. The close-on-exec it appeared to ask for still
    // holds for every seatd-obtained fd, from libseat's own receive path
    // (`recvmsg(..., MSG_CMSG_CLOEXEC)`; measured live 2026-09-13: the DRM
    // and input fds all carry the bit) -- and the bit is load-bearing, not
    // belt-and-braces: `State::spawn` provably inherits any fd lacking it,
    // pinned by `a_spawned_child_inherits_no_close_on_exec_fd`.
    let fd = session.open(path, OFlags::RDWR).map_err(|error| {
        Rejection::SessionOpen(format!("could not be opened through the session ({error})"))
    })?;

    match probe(fd.as_fd(), requested) {
        Ok(connected) => Ok(OpenGpu { fd, connected }),
        Err(reason) => {
            // Back to libseat, not merely dropped: dropping closes our fd
            // but leaves seatd holding the device open for the life of the
            // process, and leaves libseat's own fd->device table with a
            // stale entry. Failing to close is not itself a reason to stop
            // trying other devices, so it is logged, not propagated.
            if let Err(error) = session.close(fd) {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "could not hand the rejected drm device back to the session"
                );
            }
            Err(Rejection::Unusable(reason))
        }
    }
}

/// Reads KMS state through a borrowed fd -- see this module's doc for why
/// the check happens before anything takes ownership of it.
fn probe(fd: BorrowedFd<'_>, requested: Option<(u16, u16)>) -> Result<Vec<Connected>, String> {
    let device = Probe(fd);
    // The wording stops at what the kernel actually said, and says nothing
    // about *why*: the errno varies with the cause (`ENOTSUP` from a driver
    // built without `DRIVER_MODESET`, which is the Apple Silicon report;
    // `EACCES` when the path is a render node, which is what the dev VM's
    // `renderD128` returns; `EINVAL` when `--gpu` names something that is
    // not a DRM device at all, which is what an evdev node returns there),
    // so any one diagnosis appended here would be a lie for the others --
    // and a user who mistyped a path is worse off being told their GPU
    // only computes.
    // The shared, provable part is that this device cannot mode-set; what
    // the split-GPU case looks like is in `docs/tty.md`, where it can be
    // explained rather than asserted.
    let resources = device.resource_handles().map_err(|error| {
        format!("has no usable KMS pipeline -- loading its DRM resources failed ({error})")
    })?;
    // `Cached`, deliberately -- see [`Freshness`] for why a startup probe
    // does not need to force one and a hotplug re-probe absolutely does.
    // Every connector, not the first: `--tty` drives each one it can (the
    // caller matches them to CRTCs). With one connector the list is exactly
    // the one entry the old first-wins search returned.
    let connected = find_all(&device, &resources, requested, Freshness::Cached);
    if connected.is_empty() {
        return Err("has no connected connector with a usable mode".to_owned());
    }
    Ok(connected)
}

/// A borrowed fd viewed as a DRM device, for read-only KMS queries.
/// `drm`'s two device traits are both blanket-method-only over `AsFd`, so
/// the three impls below are the whole implementation.
struct Probe<'a>(BorrowedFd<'a>);

impl AsFd for Probe<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0
    }
}

impl BasicDevice for Probe<'_> {}
impl ControlDevice for Probe<'_> {}

/// Whether a connector query may answer from the mode list the kernel
/// already has, or has to make it go and look.
///
/// This is not a tuning knob, it is the difference between the hotplug path
/// working and silently doing nothing, so it is spelled out at every call
/// site rather than left as a bare `bool`. `drm`'s `get_connector(conn,
/// force_probe)` turns `force_probe` into the `count_modes` field of
/// `drm_mode_getconnector` (`drm-ffi` 0.9.1 `mode::get_connector`: `false`
/// sends `count_modes: 1`, `true` sends `0`), and the kernel's
/// `drm_mode_getconnector` only calls the driver's `fill_modes()` -- the
/// actual re-probe, which is what re-reads EDID and rebuilds the mode list
/// -- when `count_modes` is `0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Freshness {
    /// Take the kernel's cached answer.
    ///
    /// Right at startup, and only there. Two reasons, and the first is the
    /// one that makes it correct rather than merely cheaper: the staleness
    /// [`Reprobe`](Self::Reprobe) exists for begins the moment scoot takes
    /// DRM master, which is at `session.open()` -- seatd's own `SET_MASTER`
    /// happens there, *before* [`probe`] runs, not after it (see
    /// `tty/mod.rs`'s comment on that, measured on hardware). Up to that
    /// `open`, though, no userspace master has existed, so the kernel's own
    /// fbdev/fbcon client (present whenever `CONFIG_DRM_FBDEV_EMULATION` is
    /// on, which is every mainstream distro kernel) has been handling
    /// hotplug and keeping this cache current on its own. [`probe`]'s read
    /// happens microseconds after that `open`, well inside the window where
    /// nothing has yet had a chance to go stale -- so `Cached` is correct
    /// here, not just close enough.
    ///
    /// The second is that forcing here would not be free, contrary to what
    /// it might look like. It would be a real probe, not a no-op: master is
    /// per-open-file, already held on the fd `probe` borrows for the reason
    /// above, and `DeviceFd::from` later adopts that *same* open file -- so
    /// the kernel would not demote the request. It would cost one EDID read
    /// per connector examined, per candidate device walked by
    /// [`first_usable`], on the startup path.
    Cached,
    /// Make the kernel re-probe the connector before answering.
    ///
    /// The hotplug path, and the reason it works at all. Once scoot holds
    /// DRM master, the in-kernel client that would otherwise refresh the
    /// cache stops doing so -- its hotplug handler bails out when a
    /// userspace master is present -- so the cached list stays whatever it
    /// was when we took over. Reading it after a hotplug gives back the
    /// *old* modes, `tty/hotplug.rs`'s `plan` concludes nothing changed, and
    /// the display is never re-modeset. Silently: the only trace is a debug
    /// line saying the hotplug changed nothing, which is precisely the
    /// wrong answer.
    ///
    /// It is not free. A forced probe re-reads EDID over DDC on real
    /// HDMI/DP hardware -- tens of milliseconds, and longer on a marginal
    /// cable or an adapter that needs retries -- synchronously, on the
    /// calloop thread, once per connector examined. That happens on every
    /// `change` uevent for this device and on every VT-switch-back (see
    /// `tty/hotplug.rs`'s `Tty::reconfigure` callers). With more than one
    /// output that is every connector, not only the driven ones: a monitor
    /// plugged into an undriven connector is only visible to a probe of
    /// that connector, and a disconnected one answers without an EDID read.
    /// wlroots pays exactly the same cost on the same path for the same
    /// reason; there is no cheaper way to learn what a connector is
    /// actually offering now.
    Reprobe,
}

/// Every `Connected` connector with at least one mode, in the kernel's own
/// connector order, each with its mode: the one whose size is `requested`
/// (`--mode WxH`) if that connector lists one, else its `PREFERRED`-flagged
/// one if any, else its first. `--mode` applies to each connector
/// independently -- a panel that does not offer the size keeps its own
/// preferred mode while a monitor that does takes it.
///
/// Generic over the device so the same search runs on a [`Probe`] at
/// startup and on the live `DrmDevice` when a hotplug event asks what the
/// connectors say *now* (see `tty/hotplug.rs`); `resources` is passed in
/// rather than read here so [`probe`] can tell "this device has no KMS at
/// all" (the Asahi failure) apart from "this device has KMS but nothing is
/// plugged in", and so the hotplug path can read a *fresh* set rather than
/// the one cached at startup.
///
/// One small allocation per usable connector (its name and CRTC list) plus
/// the list itself, on paths that run at startup and when a cable moves.
pub(super) fn find_all(
    device: &impl ControlDevice,
    resources: &ResourceHandles,
    requested: Option<(u16, u16)>,
    freshness: Freshness,
) -> Vec<Connected> {
    search_all(resources.connectors().iter().copied(), |conn| {
        connector_mode(device, resources, conn, requested, freshness)
    })
}

/// Every connector in `connectors` that `probe` says can drive a display,
/// in the order given. The pure half of [`find_all`], split out so the
/// ordering and filtering are pinnable without a DRM device (see this
/// module's tests): `probe` stands in for [`connector_mode`].
fn search_all<T>(
    connectors: impl Iterator<Item = connector::Handle>,
    probe: impl FnMut(connector::Handle) -> Option<T>,
) -> Vec<T> {
    connectors.filter_map(probe).collect()
}

/// One connector's mode, name and reachable CRTCs, or `None` if it isn't
/// `Connected` or lists no mode at all. The per-connector half of
/// [`find_all`], split out so the hotplug path can ask about
/// one specific connector without duplicating the choice of mode.
///
/// A `requested` size the connector does not offer is a warning, not a
/// rejection: falling through to the preferred mode leaves the user with a
/// display of the wrong size, which they can read the log about, whereas
/// rejecting the connector would leave them with no display at all -- and
/// at startup, on a multi-GPU seat, would send the search on to a device
/// they did not mean.
///
/// The returned name is built here, which is one small `String` per
/// connector that turns out usable -- at most one or two per hotplug event,
/// on a path that has just spent milliseconds reading EDID, and never on
/// any per-frame or input-dispatch path. Handing back the whole
/// `connector::Info` to let the caller build it only when it logs would
/// trade this for a much larger one.
pub(super) fn connector_mode(
    device: &impl ControlDevice,
    resources: &ResourceHandles,
    conn: connector::Handle,
    requested: Option<(u16, u16)>,
    freshness: Freshness,
) -> Option<Connected> {
    let info = device
        .get_connector(conn, freshness == Freshness::Reprobe)
        .ok()?;
    if info.state() != connector::State::Connected {
        return None;
    }
    let modes = info.modes();
    // `HDMI-A-1`, not `HDMI-A` + `1`: the same spelling the kernel
    // uses in sysfs (`/sys/class/drm/card0-HDMI-A-1`) and every
    // wlroots/Smithay compositor uses for `wl_output.name`.
    let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
    let requested_mode = requested.and_then(|size| modes.iter().find(|mode| mode.size() == size));
    if let (Some((width, height)), None, false) = (requested, requested_mode, modes.is_empty()) {
        // warn!, not debug!: the size on screen is about to disagree
        // with what the user asked for, and this is the only explanation.
        // Names the connector: with several driven, two identical lines
        // would not say which screen ignored the flag.
        tracing::warn!(
            connector = %name,
            width,
            height,
            "drm: connector offers no mode of the requested size; using its preferred mode"
        );
    }
    let mode = requested_mode
        .or_else(|| {
            modes
                .iter()
                .find(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
        })
        .or_else(|| modes.first())
        .copied()?;
    let crtcs = reachable_crtcs(device, resources, info.encoders());
    let edid = connector_edid(device, conn);
    Some(Connected {
        connector: conn,
        mode,
        name,
        crtcs,
        edid,
    })
}

/// One connector's EDID summary, or `None` where there is none to read: no
/// `EDID` property on the connector, a blob id of zero (nothing behind it),
/// a blob that refuses, or bytes that are not an EDID base block (see
/// [`parse_edid`](crate::compositor::output_identity::parse_edid)).
///
/// Quiet on every one of those: a panel with no serial and a KVM hiding the
/// blob are ordinary hardware, and the name alone still identifies the
/// connector -- the `None` is the fallback working, not a failure to log.
fn connector_edid(
    device: &impl ControlDevice,
    connector: connector::Handle,
) -> Option<EdidIdentity> {
    let set = device.get_properties(connector).ok()?;
    let (handles, values) = set.as_props_and_values();
    for (handle, value) in handles.iter().zip(values.iter()) {
        let info = device.get_property(*handle).ok()?;
        if info.name().to_str().ok()? != "EDID" {
            continue;
        }
        if *value == 0 {
            return None;
        }
        let bytes = device.get_property_blob(*value).ok()?;
        return parse_edid(&bytes);
    }
    None
}

/// The CRTCs any of `encoders` can be routed to, in the device's CRTC order
/// and without repeats. An encoder that cannot be read contributes nothing:
/// a connector left with no route stays dark (and says so where it is
/// skipped), which beats guessing a CRTC the hardware may refuse at the
/// first commit.
fn reachable_crtcs(
    device: &impl ControlDevice,
    resources: &ResourceHandles,
    encoders: &[smithay::reexports::drm::control::encoder::Handle],
) -> Vec<crtc::Handle> {
    let mut reachable: Vec<crtc::Handle> = Vec::new();
    for &encoder in encoders {
        let Ok(info) = device.get_encoder(encoder) else {
            continue;
        };
        for crtc in resources.filter_crtcs(info.possible_crtcs()) {
            if !reachable.contains(&crtc) {
                reachable.push(crtc);
            }
        }
    }
    // `filter_crtcs` yields each mask in resource order, but two encoders'
    // masks interleave; resource order is the preference `crtcs::assign`
    // expects.
    reachable.sort_by_key(|crtc| {
        resources
            .crtcs()
            .iter()
            .position(|known| known == crtc)
            .unwrap_or(usize::MAX)
    });
    reachable
}

/// What to tell the user when no candidate worked, given the same
/// `explicit` [`candidates`] was called with and every `(device, reason)`
/// that failed.
///
/// The old error named one device and one OS error; this keeps that detail
/// per device rather than collapsing the list into a count, so "tried two,
/// one has no KMS and the other has nothing plugged in" stays readable.
/// Which advice goes above that list depends on [`Rejection`]: a list that
/// is *entirely* session-open failures is a seat problem, where naming a
/// device cannot help, so it is not offered.
/// An explicit device gets its own wording: there is exactly one device
/// and the user chose it, so a list of one and an offer of the flag they
/// already passed would both be noise. (It gets no seat-specific hint
/// either, unlike the list form below: one [`Rejection::SessionOpen`] on a
/// path the user typed is far more likely to be a path that does not exist
/// than a seat that is busy, and the session's own errno already says
/// which.) The wording names the surface that actually named the device --
/// `--gpu PATH` or `[tty] gpu = "PATH"` (see
/// [`ExplicitGpu::describe`]) -- so a config-file user is never told to
/// re-check a flag they never passed.
pub fn unusable_device_error(
    seat: &str,
    explicit: Option<ExplicitGpu<'_>>,
    failures: &[(PathBuf, Rejection)],
) -> String {
    if let Some(named) = explicit {
        let reason = failures
            .iter()
            .find(|(failed, _)| *failed == named.path())
            .map_or("is not usable", |(_, reason)| reason.reason());
        // Quoted, unlike the list form below where the path starts its own
        // indented line: here it sits mid-sentence, where a path with a
        // space in it -- or the empty string, which an explicit path could
        // once reach the session as -- would otherwise dissolve into the
        // prose around it. (`resolve` refuses empty paths before any
        // session is opened, so the empty case below is defensive, kept so
        // a `map_or` that produced an empty tail would still read as a
        // sentence rather than a truncated one.)
        return match named {
            ExplicitGpu::Flag(_) => format!("the device given by {} {reason}", named.describe()),
            ExplicitGpu::Config(_) => {
                format!("the device named by {} {reason}", named.describe())
            }
        };
    }
    if failures.is_empty() {
        return format!(
            "no DRM device on seat `{seat}` -- is a GPU present and assigned \
             to this seat? (`--gpu PATH` names one explicitly)"
        );
    }
    // Every candidate refused at `Session::open` says nothing about which
    // device to pick -- it is the seat that would not take this process,
    // and it refuses all of them identically. Recommending `--gpu` there
    // sends a user off re-picking devices when no device would have worked;
    // this is the likeliest real-world failure of the three, since it is
    // what running `--tty` while another compositor holds the seat does.
    let mut message = if failures.iter().all(|(_, reason)| reason.is_session_open()) {
        format!(
            "the session refused every DRM device on seat `{seat}`: the seat \
             is what failed here, not the choice of device. A seat takes one \
             client at a time -- another compositor may already hold this one \
             -- and a session that is not allowed on the seat is refused the \
             same way. Tried:"
        )
    } else {
        format!(
            "no usable DRM device on seat `{seat}`; pass `--gpu PATH` to name one \
             explicitly. Tried:"
        )
    };
    for (path, reason) in failures {
        // Writing to a String cannot fail.
        let _ = write!(message, "\n  {} {reason}", path.display());
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn unusable(reason: &str) -> Rejection {
        Rejection::Unusable(reason.to_owned())
    }

    fn refused(reason: &str) -> Rejection {
        Rejection::SessionOpen(reason.to_owned())
    }

    /// Stands in for `all_gpus` failing: any `io::Error` will do, but udev
    /// enumeration really does surface as one of these.
    fn enumeration_failed() -> io::Error {
        io::Error::new(io::ErrorKind::OutOfMemory, "udev said no")
    }

    #[test]
    fn the_primary_gpu_is_tried_first_and_only_once() {
        assert_eq!(
            order(
                Some(PathBuf::from("/dev/dri/card1")),
                paths(&["/dev/dri/card0", "/dev/dri/card1", "/dev/dri/card2"]),
            ),
            paths(&["/dev/dri/card1", "/dev/dri/card0", "/dev/dri/card2"]),
        );
    }

    #[test]
    fn without_a_primary_the_seats_own_order_stands() {
        assert_eq!(
            order(None, paths(&["/dev/dri/card0", "/dev/dri/card1"])),
            paths(&["/dev/dri/card0", "/dev/dri/card1"]),
        );
    }

    #[test]
    fn one_device_stays_one_device() {
        assert_eq!(
            order(
                Some(PathBuf::from("/dev/dri/card0")),
                paths(&["/dev/dri/card0"])
            ),
            paths(&["/dev/dri/card0"]),
        );
    }

    #[test]
    fn no_devices_at_all_is_an_empty_list_not_a_phantom_entry() {
        assert!(order(None, Vec::new()).is_empty());
    }

    #[test]
    fn a_primary_missing_from_the_seat_listing_is_still_tried() {
        // Can't happen through `candidates` -- `primary_gpu` picks from
        // `all_gpus`'s own set -- but `order` must not silently drop it if
        // Smithay's two functions ever disagree.
        assert_eq!(
            order(Some(PathBuf::from("/dev/dri/card9")), Vec::new()),
            paths(&["/dev/dri/card9"]),
        );
    }

    #[test]
    fn a_failed_fallback_listing_still_tries_the_primary() {
        // The regression this guards: before the fallback existed, only
        // `primary_gpu` had to succeed. A machine whose primary device is
        // found and works must not fail to start because the *tail*
        // enumeration -- which exists only to be tried afterwards --
        // hiccuped.
        assert_eq!(
            assemble(
                Some(PathBuf::from("/dev/dri/card0")),
                Err(enumeration_failed())
            )
            .expect("a found primary is still worth trying"),
            paths(&["/dev/dri/card0"]),
        );
    }

    #[test]
    fn a_failed_listing_with_no_primary_is_reported_as_itself() {
        // Nothing to degrade to here, and "is a GPU present?" would blame
        // the hardware for udev's failure, so the enumeration error stands.
        let error = assemble(None, Err(enumeration_failed()))
            .expect_err("no primary and no listing leaves nothing to try");
        assert_eq!(error.kind(), io::ErrorKind::OutOfMemory);
    }

    #[test]
    fn a_listing_that_works_is_ordered_as_usual() {
        assert_eq!(
            assemble(
                Some(PathBuf::from("/dev/dri/card1")),
                Ok(paths(&["/dev/dri/card0", "/dev/dri/card1"])),
            )
            .expect("nothing failed"),
            paths(&["/dev/dri/card1", "/dev/dri/card0"]),
        );
    }

    #[test]
    fn an_explicit_gpu_replaces_the_search() {
        let chosen = Path::new("/dev/dri/card3");
        assert_eq!(
            candidates("seat0", Some(ExplicitGpu::Flag(chosen)))
                .expect("explicit paths need no seat"),
            paths(&["/dev/dri/card3"]),
        );
    }

    #[test]
    fn an_explicit_gpu_from_the_config_file_replaces_the_search_too() {
        // `[tty] gpu` reaches `candidates` as the same single-candidate
        // list a `--gpu` does: explicit means exactly that device, no
        // fallback, whichever surface named it.
        let chosen = Path::new("/dev/dri/card1");
        assert_eq!(
            candidates("seat0", Some(ExplicitGpu::Config(chosen)))
                .expect("explicit paths need no seat"),
            paths(&["/dev/dri/card1"]),
        );
    }

    #[test]
    fn resolve_prefers_the_flag_over_the_config_file() {
        let flag = Path::new("/dev/dri/card0");
        let config = Path::new("/dev/dri/card1");
        assert_eq!(
            resolve(Some(flag), Some(config)),
            Ok(Some(ExplicitGpu::Flag(flag))),
            "an explicit --gpu must beat [tty] gpu the way an explicit flag should"
        );
    }

    #[test]
    fn resolve_uses_the_config_file_when_the_flag_is_absent() {
        let config = Path::new("/dev/dri/card1");
        assert_eq!(
            resolve(None, Some(config)),
            Ok(Some(ExplicitGpu::Config(config)))
        );
    }

    #[test]
    fn resolve_with_neither_source_names_no_device() {
        assert_eq!(resolve(None, None), Ok(None));
    }

    #[test]
    fn resolve_refuses_an_empty_path_from_either_source() {
        // Fail-closed, not fail-open: an explicitly-set-but-empty path can
        // never name a device, so it is a startup error rather than
        // something the session is asked to open (or, worse, silently
        // ignored in favour of the automatic pick).
        let empty = Path::new("");
        let real = Path::new("/dev/dri/card1");
        assert!(
            resolve(Some(empty), Some(real)).is_err(),
            "an empty --gpu must not silently win over a real [tty] gpu"
        );
        assert!(
            resolve(Some(empty), None).is_err(),
            "an empty --gpu must fail, not reach the session"
        );
        assert!(
            resolve(None, Some(empty)).is_err(),
            "an empty [tty] gpu must fail, not fall back to the automatic pick"
        );
        let error = resolve(None, Some(empty)).expect_err("empty [tty] gpu is an error");
        assert!(
            error.to_string().contains("[tty] gpu"),
            "the refusal must name the key, not the flag: {error}"
        );
        let error = resolve(Some(empty), None).expect_err("empty --gpu is an error");
        assert!(
            error.to_string().contains("--gpu"),
            "the refusal must name the flag: {error}"
        );
    }

    /// `first_usable`'s `open`, recording what it was asked to open so a
    /// test can prove a later candidate was never touched.
    fn opener<'a>(
        tried: &'a mut Vec<PathBuf>,
        works: &'a str,
    ) -> impl FnMut(&Path) -> Result<&'static str, Rejection> + 'a {
        move |path| {
            tried.push(path.to_owned());
            if path == Path::new(works) {
                Ok("a device")
            } else {
                Err(unusable("has no usable KMS pipeline"))
            }
        }
    }

    #[test]
    fn the_first_working_device_wins_and_nothing_after_it_is_opened() {
        let mut tried = Vec::new();
        let found = first_usable(
            paths(&["/dev/dri/card0", "/dev/dri/card1"]),
            opener(&mut tried, "/dev/dri/card0"),
        );
        assert_eq!(
            found.map(|(path, _)| path),
            Ok(PathBuf::from("/dev/dri/card0"))
        );
        assert_eq!(tried, paths(&["/dev/dri/card0"]));
    }

    #[test]
    fn a_failing_first_device_falls_through_to_the_next_one() {
        // The whole point of the item: `primary_gpu`'s pick is a compute-only
        // GPU, and the display controller is the device after it.
        let mut tried = Vec::new();
        let found = first_usable(
            paths(&["/dev/dri/card0", "/dev/dri/card1", "/dev/dri/card2"]),
            opener(&mut tried, "/dev/dri/card1"),
        );
        assert_eq!(
            found.map(|(path, _)| path),
            Ok(PathBuf::from("/dev/dri/card1"))
        );
        assert_eq!(tried, paths(&["/dev/dri/card0", "/dev/dri/card1"]));
    }

    #[test]
    fn every_device_failing_reports_every_device_in_the_order_tried() {
        let mut tried = Vec::new();
        let found = first_usable(
            paths(&["/dev/dri/card1", "/dev/dri/card0"]),
            opener(&mut tried, "/dev/dri/nothing"),
        );
        let Err(failures) = found else {
            panic!("nothing should have been accepted");
        };
        assert_eq!(
            failures,
            vec![
                (
                    PathBuf::from("/dev/dri/card1"),
                    unusable("has no usable KMS pipeline")
                ),
                (
                    PathBuf::from("/dev/dri/card0"),
                    unusable("has no usable KMS pipeline")
                ),
            ]
        );
        assert_eq!(tried, paths(&["/dev/dri/card1", "/dev/dri/card0"]));
    }

    #[test]
    fn no_candidates_opens_nothing_and_reports_no_failures() {
        let mut tried = Vec::new();
        let found = first_usable(Vec::new(), opener(&mut tried, "/dev/dri/card0"));
        assert_eq!(found.map(|(path, _)| path), Err(Vec::new()));
        assert!(tried.is_empty());
    }

    #[test]
    fn the_final_error_names_every_device_and_why_each_failed() {
        let failures = vec![
            (
                PathBuf::from("/dev/dri/card0"),
                unusable("has no KMS resources to load (os error 95)"),
            ),
            (
                PathBuf::from("/dev/dri/card1"),
                unusable("has no connected connector with a usable mode"),
            ),
        ];
        let message = unusable_device_error("seat0", None, &failures);
        assert!(message.contains("seat0"), "{message}");
        assert!(message.contains("--gpu"), "{message}");
        assert!(
            message.contains("/dev/dri/card0 has no KMS resources"),
            "{message}"
        );
        assert!(
            message.contains("/dev/dri/card1 has no connected connector"),
            "{message}"
        );
    }

    #[test]
    fn one_failing_device_reads_as_a_reason_not_a_tally() {
        let failures = vec![(
            PathBuf::from("/dev/dri/card0"),
            unusable("has no connected connector with a usable mode"),
        )];
        let message = unusable_device_error("seat0", None, &failures);
        assert!(
            message.contains("/dev/dri/card0 has no connected connector"),
            "{message}"
        );
        // The list is the message; nothing counts how long it is, so one
        // failure reads the same way two do rather than as "tried 1 of 1".
        assert_eq!(message.matches('\n').count(), 1, "{message}");
    }

    #[test]
    fn a_seat_that_refused_everything_is_not_blamed_on_device_choice() {
        // The real case: another compositor (or the user's own session on
        // another VT) already holds the seat, so libseat answers EPERM for
        // every device. `--gpu` names a device and cannot fix a seat.
        let failures = vec![
            (
                PathBuf::from("/dev/dri/card0"),
                refused("could not be opened through the session (Operation not permitted)"),
            ),
            (
                PathBuf::from("/dev/dri/card1"),
                refused("could not be opened through the session (Operation not permitted)"),
            ),
        ];
        let message = unusable_device_error("seat0", None, &failures);
        assert!(!message.contains("--gpu"), "{message}");
        assert!(
            message.contains("the seat is what failed here"),
            "{message}"
        );
        // Still says which devices were tried and what each said: the list
        // is what makes "all of them, identically" visible at all.
        assert!(
            message.contains("/dev/dri/card0 could not be opened"),
            "{message}"
        );
        assert!(
            message.contains("/dev/dri/card1 could not be opened"),
            "{message}"
        );
    }

    #[test]
    fn one_device_that_opened_and_failed_keeps_the_gpu_advice() {
        // Not all-session-open: something did open, and picking a
        // different device is exactly what might help.
        let failures = vec![
            (
                PathBuf::from("/dev/dri/card0"),
                refused("could not be opened through the session (Operation not permitted)"),
            ),
            (
                PathBuf::from("/dev/dri/card1"),
                unusable("has no connected connector with a usable mode"),
            ),
        ];
        let message = unusable_device_error("seat0", None, &failures);
        assert!(message.contains("--gpu"), "{message}");
        assert!(
            !message.contains("the seat is what failed here"),
            "{message}"
        );
    }

    #[test]
    fn no_devices_found_says_so_rather_than_listing_nothing() {
        let message = unusable_device_error("seat0", None, &[]);
        assert!(
            message.contains("no DRM device on seat `seat0`"),
            "{message}"
        );
        assert!(!message.contains("Tried:"), "{message}");
    }

    #[test]
    fn an_explicit_gpu_is_blamed_by_name_without_a_list() {
        let chosen = PathBuf::from("/dev/dri/card9");
        let failures = vec![(
            chosen.clone(),
            refused("could not be opened through the session (No such file or directory)"),
        )];
        let message = unusable_device_error(
            "seat0",
            Some(ExplicitGpu::Flag(chosen.as_path())),
            &failures,
        );
        assert_eq!(
            message,
            "the device given by `--gpu /dev/dri/card9` could not be opened \
             through the session (No such file or directory)"
        );
    }

    #[test]
    fn an_explicit_gpu_from_the_config_file_is_blamed_as_the_key_not_the_flag() {
        // A user who set `[tty] gpu` and never typed `--gpu` must not be
        // told the device came from `--gpu`: the wording names the surface
        // that actually named it.
        let chosen = PathBuf::from("/dev/dri/card1");
        let failures = vec![(
            chosen.clone(),
            unusable("has no connected connector with a usable mode"),
        )];
        let message = unusable_device_error(
            "seat0",
            Some(ExplicitGpu::Config(chosen.as_path())),
            &failures,
        );
        assert_eq!(
            message,
            "the device named by `[tty] gpu = \"/dev/dri/card1\"` \
             has no connected connector with a usable mode"
        );
        assert!(!message.contains("--gpu"), "{message}");
    }

    #[test]
    fn an_explicit_gpu_with_no_recorded_reason_still_reads_as_a_sentence() {
        // Defensive: `init` always records a reason for every candidate it
        // tried, so this is unreachable today -- but a `map_or` that
        // produced an empty tail would read as a truncated sentence.
        let chosen = PathBuf::from("/dev/dri/card9");
        assert_eq!(
            unusable_device_error("seat0", Some(ExplicitGpu::Flag(chosen.as_path())), &[]),
            "the device given by `--gpu /dev/dri/card9` is not usable"
        );
    }

    fn conn(raw: u32) -> connector::Handle {
        connector::Handle::from(std::num::NonZeroU32::new(raw).expect("connector ids start at 1"))
    }

    #[test]
    fn every_usable_connector_is_kept_in_kernel_order() {
        // The E1 generalisation: first-wins became collect-all, and the order
        // is still the kernel's -- so the first entry is exactly what the old
        // single-output search returned.
        let found = search_all([conn(52), conn(60), conn(70)].into_iter(), |c| {
            (c != conn(60)).then_some(c)
        });
        assert_eq!(found, vec![conn(52), conn(70)]);
    }

    #[test]
    fn one_usable_connector_is_the_old_single_output_answer() {
        let found = search_all([conn(52)].into_iter(), Some);
        assert_eq!(found, vec![conn(52)]);
    }

    #[test]
    fn nothing_usable_is_an_empty_list() {
        let found = search_all([conn(52), conn(70)].into_iter(), |_| None::<()>);
        assert!(found.is_empty());
    }

    #[test]
    fn an_empty_explicit_path_is_still_visible_in_the_message() {
        // Defensive: `resolve` refuses empty paths before any session is
        // opened, so this arm is unreachable through `init` today -- but a
        // `map_or` that produced an empty tail would read as a truncated
        // sentence. The quoting is what keeps the resulting sentence from
        // reading as if no path had been named at all.
        let chosen = PathBuf::new();
        assert_eq!(
            unusable_device_error("seat0", Some(ExplicitGpu::Flag(chosen.as_path())), &[]),
            "the device given by `--gpu ` is not usable"
        );
    }
}
