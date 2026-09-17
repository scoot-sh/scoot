//! `zwlr_gamma_control_manager_v1`: night-light tools (`gammastep`, `wlsunset`).
//!
//! Smithay has no gamma module at the pinned rev, so this is hand-implemented
//! against the `wlr-gamma-control-unstable-v1` server bindings from
//! `wayland-protocols-wlr` (re-exported through Smithay, so the types are the
//! same ones the rest of the compositor dispatches). The shape follows
//! Smithay's own `wlr_data_control` manager: a `GlobalData` created once in
//! [`GammaControlState::new`], a per-bound-manager `UserData`, and
//! `Dispatch2`/`GlobalDispatch2` impls that flexwm's blanket `Dispatch` (see
//! `dispatch.rs`) forwards to with no per-interface code of its own.
//!
//! ## Trust model
//!
//! The global is visible to every client (`can_view` is unconditional).
//! flexwm has no security-context support to distinguish a privileged
//! night-light daemon from any other client, so an allow-list would be
//! theatre -- the same rationale as the session-lock global, documented in
//! `README.md`'s trust note.
//!
//! ## What `set_gamma` carries
//!
//! The protocol XML says each ramp entry is a "16-byte unsigned integer".
//! That is a documentation typo: `wlroots`, `niri` and every client treat
//! entries as 16-bit, and the fd holds `3 * gamma_size` little-endian `u16`
//! ramps (red, green, blue), i.e. `6 * gamma_size` bytes total. This module
//! reads exactly that, and rejects anything else with `invalid_gamma`.
//!
//! The fd is read with a bounded non-blocking read, not `mmap`: a client can
//! truncate or never fill what it passed, and `mmap` would turn that into a
//! `SIGBUS` inside the compositor (Smithay's `SIGBUS` handler only covers its
//! own `wl_shm` pools) or an unbounded mapping. The read is bounded at
//! `6 * size` plus one 4096-byte chunk -- and `size` itself is clamped at
//! construction -- so a malicious fd can cost neither memory nor a blocked
//! event loop: a pipe that never delivers simply reads short (or empty) and
//! is refused.
//!
//! ## Backends
//!
//! The ramp is accepted on every backend. It is *applied* only
//! under `--tty`, where there is a real CRTC gamma LUT to push it to (see
//! [`Tty::set_gamma_ramp`](super::tty::Tty::set_gamma_ramp)). Under
//! `--headless`/`--nested` there is no hardware LUT: the request succeeds
//! but nothing on screen changes -- and an IPC
//! screenshot reads the framebuffer, which is pre-LUT, so it shows the
//! unmodified frame either way. `README.md` says this where a user will find
//! it.
//!
//! Any DRM failure while applying (no master after a VT switch, a driver
//! that refuses the size) retires the control with a `failed` event and the
//! session keeps running -- per the protocol, "setting the gamma tables
//! failed" invalidates the object, it does not take down the compositor.
//!
//! Lock-session interaction: none. Gamma is output-level hardware state, not
//! a surface, so a control keeps working while the session is locked --
//! including a night-light daemon adjusting the temperature over a lock
//! screen, which is the ordinary case, not a bypass (it changes no pixels'
//! content, only their color temperature).

use std::io::Read;
use std::os::unix::io::{AsFd, OwnedFd};

use smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::{
    zwlr_gamma_control_manager_v1, zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
    zwlr_gamma_control_v1, zwlr_gamma_control_v1::ZwlrGammaControlV1,
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, New, Resource};
use smithay::wayland::{Dispatch2, GlobalDispatch2};

use super::State;

#[cfg(test)]
mod tests;

/// The near-universal DRM default, used wherever the real CRTC size is
/// unknown: under `--headless`/`--nested` (no CRTC at all), and under `--tty`
/// when the query fails or reports something unusable. See
/// [`Tty::gamma_size`](super::tty::Tty::gamma_size) for the query and its
/// clamp.
pub(super) const FALLBACK_GAMMA_SIZE: u32 = 256;

/// Holds the `zwlr_gamma_control_manager_v1` global alive and tracks the one
/// live control. flexwm has exactly one output, so "at most one live control
/// per output" is a single [`Option`]: a second `get_gamma_control` fails the
/// old control and takes its place.
pub(crate) struct GammaControlState {
    /// Held only to keep the manager global alive -- like
    /// `output_manager_state`, nothing reads this field again after `new`.
    #[allow(dead_code)]
    manager_global: GlobalId,
    /// Entries per ramp. Fixed at construction on headless/nested
    /// ([`FALLBACK_GAMMA_SIZE`]); overwritten from the real CRTC by
    /// `tty::init` once the DRM surface exists.
    size: u32,
    /// The live control, if any. Compared by object identity on destroy, so
    /// tearing down a superseded (already `failed`) control cannot clear a
    /// newer one. This is the only ramp record: `None` is the default linear
    /// ramp, `Some` is a client ramp still in force -- there is deliberately
    /// no copy of the ramp itself, nothing reads it back (see `set_gamma`).
    current: Option<ZwlrGammaControlV1>,
}

impl GammaControlState {
    pub(super) fn new(display: &DisplayHandle) -> Self {
        let manager_global = display
            .create_global::<State, ZwlrGammaControlManagerV1, GammaControlManagerGlobalData>(
                1,
                GammaControlManagerGlobalData,
            );
        Self {
            manager_global,
            size: FALLBACK_GAMMA_SIZE,
            current: None,
        }
    }

    /// Records the real CRTC size once `--tty` knows it. Called once from
    /// `tty::init`, never again: the CRTC (and its LUT length) is fixed for
    /// the process's life.
    pub(super) fn set_size(&mut self, size: u32) {
        self.size = size;
    }

    /// The expected `set_gamma` fd length in bytes: three ramps of `size`
    /// little-endian `u16` entries.
    fn expected_len(&self) -> usize {
        self.size as usize * 6
    }

    /// The default linear ramp for the current size: entry `i` of each ramp
    /// is `i * 65535 / (size - 1)`.
    fn linear_ramp(&self) -> Vec<u16> {
        linear_ramp(self.size)
    }
}

/// Entries for one linear ramp of `size`: `i * 65535 / (size - 1)`.
///
/// Free rather than inline in the method above so the construction has one
/// home with its divide-by-zero guard in one place: `size < 2` would divide
/// by zero, and every size that reaches here is clamped to at least 2 first
/// (see `Tty::gamma_size`), so this asserts rather than silently producing
/// a flat ramp.
pub(super) fn linear_ramp(size: u32) -> Vec<u16> {
    assert!(size >= 2, "gamma size {size} cannot hold a linear ramp");
    (0..size)
        .map(|i| ((u64::from(i) * 65535) / u64::from(size - 1)) as u16)
        .collect()
}

/// Global data for the manager. Empty: every client may bind (see the module
/// doc's trust model), so there is no filter to carry.
pub(super) struct GammaControlManagerGlobalData;

impl GlobalDispatch2<ZwlrGammaControlManagerV1, State> for GammaControlManagerGlobalData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrGammaControlManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, GammaControlManagerUserData);
    }
}

/// Per-bound-manager user data. Empty: all state lives in
/// [`GammaControlState`], reached through `&mut State`.
pub(super) struct GammaControlManagerUserData;

impl Dispatch2<ZwlrGammaControlManagerV1, State> for GammaControlManagerUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrGammaControlManagerV1,
        request: <ZwlrGammaControlManagerV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } => {
                get_gamma_control(state, data_init, id, &output);
            }
            zwlr_gamma_control_manager_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }
}

/// Per-control user data. Empty, for the same reason as the manager's: the
/// live control is tracked in [`GammaControlState::current`].
pub(super) struct GammaControlUserData;

impl Dispatch2<ZwlrGammaControlV1, State> for GammaControlUserData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZwlrGammaControlV1,
        request: <ZwlrGammaControlV1 as Resource>::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_gamma_control_v1::Request::SetGamma { fd } => {
                // Stale controls -- superseded by a transfer, or already
                // failed -- stop affecting anything: the transfer sent them
                // `failed`, and anything they send now is a client bug, not
                // state to apply. (A client that destroys and re-creates gets
                // a fresh object, which *is* current, so this only drops
                // requests on dead objects.)
                let is_current = state
                    .gamma_control
                    .current
                    .as_ref()
                    .is_some_and(|current| current == resource);
                if !is_current {
                    return;
                }
                set_gamma(state, resource, fd);
            }
            zwlr_gamma_control_v1::Request::Destroy => (),
            _ => unreachable!(),
        }
    }

    fn destroyed(&self, state: &mut State, _client: ClientId, resource: &ZwlrGammaControlV1) {
        // A disconnect destroys the object without the request above ever
        // running -- same restore either way. The identity check matters:
        // destroying a superseded control must not restore the default over
        // the live one's ramp.
        let is_current = state
            .gamma_control
            .current
            .as_ref()
            .is_some_and(|current| current == resource);
        if is_current {
            restore_default(state);
        }
    }
}

/// Creates the per-output control for `get_gamma_control`.
///
/// Always initializes `id` and always answers on the new object -- `gamma_size`
/// for flexwm's own output, `failed` for anything else (an output that went
/// away, or a request that arrived before `headless::init` created the one
/// output there is). Initializing-then-failing, rather than posting a protocol
/// error without initializing, keeps this on the protocol's own rails: the
/// "output doesn't support gamma tables" case is exactly what `failed` is
/// for, and it avoids the never-initialized-object shape `dispatch.rs`
/// documents for the `wl_shm` guards.
fn get_gamma_control(
    state: &mut State,
    data_init: &mut DataInit<'_, State>,
    id: New<ZwlrGammaControlV1>,
    output: &smithay::reexports::wayland_server::protocol::wl_output::WlOutput,
) {
    let known = state
        .output
        .as_ref()
        .is_some_and(|known| known.owns(output));
    let control: ZwlrGammaControlV1 = data_init.init(id, GammaControlUserData);
    if !known {
        control.failed();
        return;
    }
    // Exclusivity transfer: at most one live control per output. The old one
    // is told it lost control and stops affecting anything (see the
    // `is_current` gates); the hardware keeps showing its ramp until the new
    // control sets one or goes away, which is what wlroots does too -- there
    // is no "no control" state that would restore the default mid-transfer.
    if let Some(old) = state.gamma_control.current.replace(control.clone()) {
        old.failed();
    }
    control.gamma_size(state.gamma_control.size);
}

/// Reads, validates, stores and (on `--tty`) applies one `set_gamma`.
///
/// Anything about the fd that is not exactly `3 * size` little-endian `u16`
/// entries -- short, long, empty, unreadable -- is `invalid_gamma`
/// (protocol error value 1), which disconnects the client. That is the
/// protocol's own answer and what `gammastep` expects from a compositor that
/// cannot use what it sent.
fn set_gamma(state: &mut State, resource: &ZwlrGammaControlV1, fd: OwnedFd) {
    let expected = state.gamma_control.expected_len();
    let bytes = match read_bounded(fd, expected) {
        Some(bytes) if bytes.len() == expected => bytes,
        _ => {
            resource.post_error(
                zwlr_gamma_control_v1::Error::InvalidGamma,
                format!(
                    "gamma table must be exactly three ramps of {} u16 entries ({} bytes)",
                    state.gamma_control.size, expected,
                ),
            );
            return;
        }
    };
    // Nowhere to apply to outside `--tty` (headless/nested have no LUT):
    // the length check above is the whole validation and the request
    // succeeds. The accepted bytes are deliberately not kept -- nothing
    // reads a stored ramp back (a future "report the current ramp" surface
    // would need a real consumer first; see the follow-ups record), so
    // keeping one would retain ~4.6 KB per ramp until destroy for nobody.
    let Some(tty) = state.tty.as_ref() else {
        return;
    };
    let ramp: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    // `chunks_exact(2)` on an even length is exact by construction (`expected`
    // is a multiple of 6), so this always holds -- and a short ramp here
    // would misalign the three channels, which is worth a hard guarantee
    // rather than a silent slice.
    debug_assert_eq!(ramp.len(), state.gamma_control.size as usize * 3);

    let size = state.gamma_control.size as usize;
    if tty
        .set_gamma_ramp(&ramp[..size], &ramp[size..2 * size], &ramp[2 * size..])
        .is_err()
    {
        // The object is dead from here: `failed` means "no longer valid"
        // and the client should destroy it. The hardware keeps showing the
        // last ramp it accepted; there is no record to update, so nothing
        // can go stale.
        state.gamma_control.current = None;
        resource.failed();
    }
}

/// Restores the default linear ramp when the current control goes away --
/// explicit destroy or client disconnect.
fn restore_default(state: &mut State) {
    state.gamma_control.current = None;
    if let Some(tty) = state.tty.as_ref() {
        // Nothing to signal failure on: the object this would be about is
        // already gone. A failed restore leaves the last ramp on the hardware
        // rather than crashing the session; the next control starts clean
        // either way.
        let ramp = state.gamma_control.linear_ramp();
        let size = state.gamma_control.size as usize;
        if tty
            .set_gamma_ramp(&ramp[..size], &ramp[size..2 * size], &ramp[2 * size..])
            .is_err()
        {
            tracing::warn!("could not restore the default gamma ramp");
        }
    }
}

/// Reads a bounded prefix of `fd`, without ever blocking the event loop.
///
/// `NONBLOCK` is set first: for a memfd or regular file (what every real
/// client sends) it changes nothing, and for a pipe or socket (what a
/// malicious or broken client sends) it turns "wait for EOF" into "return
/// what is there", so a drip-fed fd reads short and is refused instead of
/// parking the compositor. Reads stop once past `limit` -- anything beyond it
/// is the oversize detector, and stopping there bounds the allocation to at
/// most `limit + 4096` (4096-byte chunk reads) no matter what the fd claims
/// to hold.
///
/// Takes the fd by value: it arrived over the socket for this one read, and
/// nothing else ever needs it afterwards.
///
/// The read is *positioned* at offset zero, not sequential. The fd arrives as
/// an `SCM_RIGHTS` duplicate sharing the sender's open file description --
/// including its offset -- so a client that `write()`s its ramp leaves the
/// offset at EOF while an mmap-based compositor (wlroots) reads from offset
/// zero regardless. Matching that means accepting what wlroots accepts;
/// trusting the offset would refuse ramps wlroots applies. Positioned reads
/// also leave the shared offset undisturbed. Pipes and sockets don't support
/// them (`ESPIPE`): those fall back to a sequential read, which for an
/// unseekable fd starts wherever the fd is.
///
/// Returns `None` when the fd cannot be read at all; the caller treats that
/// the same as a short read (refuse), because from the protocol's point of
/// view "no table" and "a truncated table" are the same request.
fn read_bounded(fd: OwnedFd, limit: usize) -> Option<Vec<u8>> {
    use rustix::fs::{OFlags, fcntl_setfl};

    let file = std::fs::File::from(fd);
    if fcntl_setfl(file.as_fd(), OFlags::NONBLOCK).is_err() {
        return None;
    }
    read_positioned(&file, limit).or_else(|| read_sequential(&file, limit))
}

/// Positioned read of at most `limit + 4096` bytes from offset zero: whole
/// 4096-byte chunks are appended until past `limit`, so one chunk overshoots
/// the stop. Returns `None` on an unreadable fd -- or on `ESPIPE`, which is
/// the caller's cue to try [`read_sequential`] instead rather than a refusal.
/// ([`read_sequential`] is exact at `limit + 1` -- `take` stops mid-chunk --
/// so its own bound comment needs no such correction.)
fn read_positioned(file: &std::fs::File, limit: usize) -> Option<Vec<u8>> {
    use std::os::unix::fs::FileExt;

    let mut bytes = Vec::new();
    let mut position = 0u64;
    let mut chunk = [0u8; 4096];
    loop {
        if bytes.len() > limit {
            break;
        }
        match file.read_at(&mut chunk, position) {
            Ok(0) => break,
            Ok(count) => {
                bytes.extend_from_slice(&chunk[..count]);
                position += count as u64;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
    Some(bytes)
}

/// Sequential read of at most `limit + 1` bytes, for fds positioned reads
/// cannot serve. Returns `None` when nothing can be read -- including the
/// `WouldBlock` a never-delivering pipe answers with, which is what keeps
/// this from parking the event loop.
fn read_sequential(file: &std::fs::File, limit: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    // Bounded before reading, not after: `take` stops the read at
    // `limit + 1`, so the allocation never exceeds it either.
    match std::io::copy(&mut file.take(limit as u64 + 1), &mut bytes) {
        Ok(_) => Some(bytes),
        Err(_) => None,
    }
}
