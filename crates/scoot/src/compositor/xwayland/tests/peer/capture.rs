//! The peer's screen-capture half: an `ext-image-copy-capture-v1` client
//! (what `grim` and the portal speak) capturing one output into a real
//! `wl_shm` buffer, and reading its own buffer back out of the memfd -- so a
//! test asserts on the bytes the client was handed, not on anything
//! compositor-side.
//!
//! Everything is bound lazily, on the first capture: the other live suites'
//! peer binds no `wl_output` and no capture manager, exactly as before.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use wayland_client::protocol::{wl_output, wl_registry, wl_shm};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1 as sources,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1 as frame, ext_image_copy_capture_manager_v1 as manager,
    ext_image_copy_capture_session_v1 as session,
};

use super::{Ack, Peer};
use crate::compositor::test_support::wait_for;

/// The capture steps.
#[derive(Debug)]
pub(in crate::compositor::xwayland::tests) enum CaptureStep {
    /// Capture the output the registry listed `n`th (0 is the primary) into
    /// an `Argb8888` buffer and answer [`Ack::Captured`] with its bytes --
    /// `Err` naming why when the session was stopped or the frame failed.
    Output(usize),
}

/// What a buffer is filled with before a capture: a pixel still holding it
/// afterwards is one the compositor never wrote. Nothing these suites draw
/// is this color.
const SENTINEL: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

#[derive(Default)]
pub(super) struct Capture {
    /// Every `wl_output` global, `(name, version)`, in registry order.
    pub(super) output_names: Vec<(u32, u32)>,
    pub(super) sources_name: Option<u32>,
    pub(super) manager_name: Option<u32>,
    outputs: Vec<wl_output::WlOutput>,
    sources: Option<sources::ExtOutputImageCaptureSourceManagerV1>,
    manager: Option<manager::ExtImageCopyCaptureManagerV1>,
    /// The current session's constraint batch.
    size: Option<(u32, u32)>,
    dones: u32,
    stopped: bool,
    /// The current frame's outcome: `Some(true)` ready, `Some(false)` failed.
    outcome: Option<bool>,
}

pub(super) fn step(
    peer: &mut Peer,
    queue: &mut EventQueue<Peer>,
    registry: &wl_registry::WlRegistry,
    step: CaptureStep,
) -> Result<Ack, String> {
    match step {
        CaptureStep::Output(index) => Ok(Ack::Captured(capture(peer, queue, registry, index)?)),
    }
}

/// Binds what has not been bound yet: every `wl_output` the registry has
/// listed and the two capture managers.
fn bind(peer: &mut Peer, registry: &wl_registry::WlRegistry, qh: &QueueHandle<Peer>) {
    let capture = &mut peer.capture;
    for &(name, version) in &capture.output_names[capture.outputs.len()..] {
        capture
            .outputs
            .push(registry.bind(name, version.min(4), qh, ()));
    }
    if capture.sources.is_none()
        && let Some(name) = capture.sources_name
    {
        capture.sources = Some(registry.bind(name, 1, qh, ()));
    }
    if capture.manager.is_none()
        && let Some(name) = capture.manager_name
    {
        capture.manager = Some(registry.bind(name, 1, qh, ()));
    }
}

/// One whole capture of `outputs[index]`: source, session, constraints, one
/// frame into a fresh sentinel-filled buffer, read back. The session and its
/// frame are destroyed before returning, so captures do not accumulate.
fn capture(
    peer: &mut Peer,
    queue: &mut EventQueue<Peer>,
    registry: &wl_registry::WlRegistry,
    index: usize,
) -> Result<Result<Vec<u8>, String>, String> {
    let qh = queue.handle();
    bind(peer, registry, &qh);
    let output = peer
        .capture
        .outputs
        .get(index)
        .cloned()
        .ok_or_else(|| format!("no wl_output at registry index {index}"))?;
    let sources = peer
        .capture
        .sources
        .clone()
        .ok_or("no ext_output_image_capture_source_manager_v1")?;
    let manager = peer
        .capture
        .manager
        .clone()
        .ok_or("no ext_image_copy_capture_manager_v1")?;
    let shm = peer.shm.clone().ok_or("no wl_shm")?;
    peer.capture.size = None;
    peer.capture.dones = 0;
    peer.capture.stopped = false;
    peer.capture.outcome = None;

    let source = sources.create_source(&output, &qh, ());
    let session = manager.create_session(&source, manager::Options::empty(), &qh, ());
    source.destroy();
    wait_for(queue, peer, "a capture constraint batch", |peer| {
        (peer.capture.dones > 0 || peer.capture.stopped).then_some(())
    })?;
    if peer.capture.stopped {
        session.destroy();
        return Ok(Err("the capture session was stopped".to_owned()));
    }
    let (width, height) = peer.capture.size.ok_or("a batch with no buffer size")?;
    let (width, height) = (
        i32::try_from(width).map_err(|e| e.to_string())?,
        i32::try_from(height).map_err(|e| e.to_string())?,
    );
    let stride = width.checked_mul(4).ok_or("a stride past i32")?;
    let bytes = stride.checked_mul(height).ok_or("a buffer past i32")?;
    let len = usize::try_from(bytes).map_err(|e| e.to_string())?;
    let fd = rustix::fs::memfd_create("scoot-xwayland-capture", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::from(fd);
    let fill: Vec<u8> = SENTINEL.iter().copied().cycle().take(len).collect();
    file.write_all(&fill).map_err(|e| e.to_string())?;
    let pool = shm.create_pool(file.as_fd(), bytes, &qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &qh, ());
    pool.destroy();

    let frame = session.create_frame(&qh, ());
    frame.attach_buffer(&buffer);
    frame.damage_buffer(0, 0, width, height);
    frame.capture();
    let ready = wait_for(queue, peer, "a capture frame's outcome", |peer| {
        peer.capture.outcome
    })?;
    frame.destroy();
    session.destroy();
    buffer.destroy();
    if !ready {
        return Ok(Err("the capture frame failed".to_owned()));
    }
    let mut pixels = vec![0; len];
    file.read_exact_at(&mut pixels, 0)
        .map_err(|e| e.to_string())?;
    Ok(Ok(pixels))
}

impl Dispatch<session::ExtImageCopyCaptureSessionV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        _: &session::ExtImageCopyCaptureSessionV1,
        event: session::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            session::Event::BufferSize { width, height } => {
                peer.capture.size = Some((width, height));
            }
            session::Event::Done => peer.capture.dones += 1,
            session::Event::Stopped => peer.capture.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<frame::ExtImageCopyCaptureFrameV1, ()> for Peer {
    fn event(
        peer: &mut Self,
        _: &frame::ExtImageCopyCaptureFrameV1,
        event: frame::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            frame::Event::Ready => peer.capture.outcome = Some(true),
            frame::Event::Failed { .. } => peer.capture.outcome = Some(false),
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(Peer: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(Peer: ignore sources::ExtOutputImageCaptureSourceManagerV1);
wayland_client::delegate_noop!(Peer: ignore ext_image_capture_source_v1::ExtImageCaptureSourceV1);
wayland_client::delegate_noop!(Peer: ignore manager::ExtImageCopyCaptureManagerV1);
