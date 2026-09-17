//! A minimal, honest `zwp_linux_dmabuf_v1` advertisement.
//!
//! This is the advertisement half of screen capture, not capture work: it
//! exists because quickshell 0.3.1's `WlBufferManager::isReady` requires real
//! `zwp_linux_dmabuf_v1` feedback events before *any* `ScreencopyView` --
//! including the `ext-image-copy-capture-v1` output path
//! [`screencopy`](super::screencopy) already serves over `wl_shm` -- will
//! create its capture context. Without this global every quickshell
//! `ScreencopyView` is blank here despite its protocol working (`grim`
//! proves it). See
//! `docs/backlog/protocols/screencopy-shell-thumbnails-fallback.md` for the
//! measurement that established this, and
//! `docs/backlog/protocols/linux-dmabuf-advertisement.md` for the item.
//!
//! ## What is advertised, exactly
//!
//! Smithay's `DmabufState` + `DmabufHandler` at the pinned rev, one global via
//! `create_global_with_default_feedback` -- which is what fixes the global's
//! version at **6**: feedback (`get_default_feedback`, v4) needs a v4+
//! global, and Smithay advertises 6 whenever default feedback is present, 3
//! when it is not. There is no version knob to turn here, and none is
//! needed: the probe measured quickshell binding at v5 and mesa's egl queues
//! at v4 against this same v6 global, and both were served the same feedback.
//! A client binding v3 or lower never sees feedback at all -- Smithay answers
//! it with `format`/`modifier` events derived from the main tranche instead,
//! which describe the same two formats.
//!
//! The default feedback names this machine's real scanout `dev_t` as
//! `main_device` ([`main_device`]: `/dev/dri/card0`, else `renderD128`, else
//! `0`, each logged once at startup) and exactly the formats the shm capture
//! pipeline actually serves ([`DMABUF_FORMATS`]: `Xrgb8888` then `Argb8888`)
//! with the `LINEAR` layout shm buffers really have.
//!
//! ## Honesty, in one sentence
//!
//! Every datum in the advertisement is true -- a real DRM `dev_t`, the two
//! pixel formats the shm pipeline actually serves, the layout shm buffers
//! really have -- but its tranche structure implies a dmabuf *import*
//! capability flexwm does not have; any import attempt is answered `failed`,
//! the protocol's own "cannot import for implementation-dependent reasons",
//! which is the only truthful answer a pixman/shm compositor has.
//!
//! Two things deliberately *not* done in the name of honesty, because the
//! measurement record explains why each backfires: no empty format table (a
//! client `mmap`s the table Smithay always sends, and a zero-length `mmap`
//! is `EINVAL` -- quickshell turns that into `qFatal`, i.e. advertising
//! nothing truthful would abort the client rather than merely not flip it),
//! and no omitted `main_device` (same abort hazard). One real format is the
//! most honest shape that does not crash the client it is for.
//!
//! ## Trust model
//!
//! No client filter, the same deliberate consistency as
//! [`screencopy`](super::screencopy)'s capture globals: flexwm has no
//! security-context support, so an allow-list would be theatre. This global
//! hands out no pixels by itself -- it describes formats and answers `failed`
//! -- so it extends that trust note rather than widening it. See `README.md`.
//!
//! ## Edge cases, stated rather than re-derived
//!
//! - **`create_params` with garbage is `failed` or a protocol error, never a
//!   panic.** A valid format/modifier/plane set reaches
//!   [`DmabufHandler::dmabuf_imported`](smithay::wayland::dmabuf::DmabufHandler::dmabuf_imported),
//!   which answers `failed`. An unknown format never gets that far --
//!   Smithay posts `InvalidFormat`/`InvalidDimensions`/`OutOfBounds` on the
//!   params object, which disconnects that client and no one else. Both are
//!   covered in `dmabuf/tests.rs`.
//! - **Bind/unbind storms cost nothing here.** Feedback is built once, at
//!   startup; Smithay re-sends the stored copy to each new `get_default_feedback`
//!   without calling back into this module. There is no per-bind work to
//!   storm.
//! - **Hotplug and mode changes need no re-send.** The feedback names the DRM
//!   *device* (`/dev/dri/card0`), not a connector or a mode, and the formats
//!   are the shm pipeline's, which no hotplug changes -- so
//!   `set_default_feedback` is never called. If a future renderer grows real
//!   per-connector tranche preferences, that is when this paragraph stops
//!   being true.
//! - **No-DRM-node logging is once per boot, not per frame.** The
//!   `main_device = 0` fallback is logged where it is chosen, in
//!   [`advertise`], which runs once in [`Screencopy::new`](super::screencopy::Screencopy).
//!   Per-attempt import refusals are `debug!` for the same reason in the
//!   other direction: every dmabuf-capable client tries once at startup, so
//!   anything louder would spam the log per client launch.
//! - **A feedback build failure skips the global rather than half-advertising.**
//!   `DmabufFeedbackBuilder::build` fails only if the format-table memfd
//!   cannot be created; [`advertise`] then logs and returns a state with no
//!   global, and the compositor runs exactly as before this module existed.
//!   There is deliberately no `DmabufGlobal` handle stored anywhere: the
//!   display owns the advertisement and the state owns the feedback, so a
//!   bare `DmabufState` is everything a static advertisement needs to keep.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Format, Fourcc, Modifier};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::dmabuf::{
    DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
};

use super::State;

/// The dmabuf formats this compositor advertises, in the order a client sees
/// them in the feedback table.
///
/// Deliberately the same two [`screencopy`](super::screencopy) serves captures
/// in, in the same order (`Xrgb8888` first, for the same translucent-background
/// reason that module's doc gives): these are the formats a client falling
/// back to shm will actually allocate, so the table describes the fallback it
/// will take rather than a dmabuf path that does not exist. Kept as `Fourcc`
/// rather than derived from `screencopy`'s `wl_shm` list so there is no format
/// mapping to get wrong; `dmabuf/tests.rs` pins the two lists to each other.
const DMABUF_FORMATS: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];

/// Creates the `zwp_linux_dmabuf_v1` global, or returns a bare state when the
/// feedback cannot be built (see the module doc: no global beats a
/// half-advertised one).
///
/// Runs once, from [`Screencopy::new`](super::screencopy::Screencopy) -- never
/// per bind, per frame or per hotplug event.
pub(super) fn advertise(dh: &DisplayHandle) -> DmabufState {
    let mut state = DmabufState::new();
    let device = main_device();
    let formats = DMABUF_FORMATS.iter().map(|code| Format {
        code: *code,
        modifier: Modifier::Linear,
    });
    match DmabufFeedbackBuilder::new(device, formats).build() {
        Ok(feedback) => {
            state.create_global_with_default_feedback::<State>(dh, &feedback);
        }
        Err(error) => {
            tracing::warn!(
                %error,
                "dmabuf feedback table could not be built; running without zwp_linux_dmabuf_v1"
            );
        }
    }
    state
}

/// The `main_device` for default feedback: this machine's real scanout
/// `dev_t`.
///
/// `/dev/dri/card0` first, `renderD128` where there is no primary node, `0`
/// where there is no DRM node at all -- plausibly *the* production shape on
/// GPU-less containers -- each logged once, here, at startup. `0` is what the
/// protocol reserves for "no device", so the fallback ladder degrades to the
/// spec's own answer rather than to a guess.
fn main_device() -> libc::dev_t {
    /// Bound to `PathBuf` (rather than `&str`) so the ladder below reads as
    /// data, not as three near-identical `metadata` calls.
    const CARD0: &str = "/dev/dri/card0";
    const RENDER: &str = "/dev/dri/renderD128";
    let (device, source) = main_device_from(&PathBuf::from(CARD0), &PathBuf::from(RENDER));
    tracing::info!(device, source, "dmabuf feedback main device");
    device
}

/// The ladder [`main_device`] logs, split out so a test can drive it with
/// paths it controls. Returns the device and which rung answered, for the
/// log line above.
fn main_device_from(card0: &Path, render: &Path) -> (libc::dev_t, &'static str) {
    if let Some(device) = node_rdev(card0) {
        return (device, "/dev/dri/card0");
    }
    if let Some(device) = node_rdev(render) {
        return (device, "/dev/dri/renderD128");
    }
    (0, "no DRM node")
}

/// The `rdev` of `path`, or `None` when it names nothing that can be
/// `stat`ed. A path that exists but is not a device node (a regular file, a
/// directory) reports an `rdev` of 0, which is indistinguishable from -- and
/// therefore correctly handled as -- "no device".
fn node_rdev(path: &Path) -> Option<libc::dev_t> {
    let rdev = std::fs::metadata(path).ok()?.rdev();
    // `rdev()` is 0 for non-device files; only a real device node answers the
    // question this ladder asks. But `Some(0)` must still mean "answered":
    // returning `None` for a present-but-not-device path would fall through
    // to the next rung and log the wrong source.
    Some(rdev)
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.screencopy.dmabuf
    }

    /// Refuses every dmabuf import with the protocol's own `failed`.
    ///
    /// flexwm renders on the CPU with pixman and has no GPU or dma-buf path,
    /// so there is nothing an import could bind into -- `failed` ("cannot
    /// import for implementation-dependent reasons") is the only truthful
    /// answer, and a client that gets it takes its shm fallback, which is
    /// the path this compositor actually serves. Logged at `debug`: every
    /// dmabuf-capable client tries once at startup, so anything louder would
    /// log per client launch (see the module doc).
    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        tracing::debug!(
            format = ?dmabuf.format().code,
            modifier = ?dmabuf.format().modifier,
            "client attempted a dmabuf import; answering failed (this compositor has no dmabuf path)"
        );
        notifier.failed();
    }
}

#[cfg(test)]
mod tests;
