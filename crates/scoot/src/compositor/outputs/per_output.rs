//! Phase A: every output composites, captures and controls its own strip.
//!
//! The wrong-pixels guard, as bytes on the wire and in buffers rather than
//! as fields: a bar mapped on the second output must appear in the second
//! output's framebuffer, its screenshots and its screen captures -- and must
//! not appear in the first output's -- while gamma and frame callbacks work
//! per output. A test that asserted on compositor-side ids would pass just
//! as happily against a version that drew the right ids' wrong pixels.
//!
//! One client binds everything (layer shell, both capture managers, the
//! gamma manager) so a single script can put content on one output and read
//! both back. Like the other real-client suites, these need a writable
//! `$XDG_RUNTIME_DIR`.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use scoot_core::OutputId;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1,
    ext_image_copy_capture_session_v1,
};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1, zwlr_gamma_control_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::compositor::decorations::Appearance;
use crate::compositor::headless::{self, OUTPUT_NAME};
use crate::compositor::test_support::{Harness, contains, pixel, wait_for};

/// The framebuffer each output renders into. Square and small: every
/// assertion below is a pixel coordinate.
pub(super) const CANVAS: i32 = 200;
/// A bar's height: full width, fixed rows at the top.
const BAR_HEIGHT: i32 = 24;

/// A bar's colour, as the BGRA bytes a pixman `Argb8888` buffer holds them
/// in -- distinct in every channel from the default background, so no
/// assertion can pass by accident against an undrawn screen.
pub(super) const BAR_BGRA: [u8; 4] = [0xE0, 0x20, 0x20, 0xFF];

/// What a capture buffer is pre-filled with: any pixel still holding this
/// after a capture is one the compositor never wrote.
const SENTINEL: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

/// One instruction for the client thread. `output` is an index into the
/// `wl_output` globals in registry order -- 0 is the primary output.
pub(super) enum Step {
    /// Map a solid-colour bar on `outputs[output]`, returning its index.
    BarOn { output: usize, color: [u8; 4] },
    /// Request a frame callback on bar `bar`.
    RequestBarFrame { bar: usize },
    /// Report how many `done` events each requested frame callback has seen.
    BarFrames,
    /// Capture `outputs[output]` end to end: source, session, constraints,
    /// one frame into a fresh buffer, read back out of the memfd.
    CaptureOn { output: usize },
    /// Park a capture on `outputs[output]` and return without waiting --
    /// for the cases where "the compositor has *not* answered yet" is the
    /// assertion, or the client goes away first.
    CaptureNoWait { output: usize },
    /// Ask `outputs[output]` for a gamma control and report what it said,
    /// then destroy the control.
    GammaOn { output: usize },
    /// The same, but keep the control alive and report its slot.
    GammaHold { output: usize },
    /// Report a held control's `(gamma_size, failed)` as last seen.
    GammaState { held: usize },
    /// Report what the client has been told about outputs and the objects
    /// on them: which layer surfaces were `closed`, whether the live capture
    /// session was `stopped`, which `wl_output` globals were removed.
    Removals,
    /// Bind the `wl_output` global at registry index `output` again -- the
    /// shape of a bind that was already in flight when the output went away.
    Rebind { output: usize },
}

/// What the client answers a [`Step`] with.
pub(super) enum Ack {
    Done,
    Bar(usize),
    Frames(Vec<u32>),
    Captured(Capture),
    Gamma { size: Option<u32>, failed: bool },
    Held(usize),
    GammaState { size: Option<u32>, failed: bool },
    Removals(Removals),
}

/// What [`Step::Removals`] reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Removals {
    /// `closed` per layer surface, in creation order.
    pub(super) bars_closed: Vec<bool>,
    /// Whether the live capture session saw `stopped`.
    pub(super) capture_stopped: bool,
    /// The registry indices (0 = the primary) of every `wl_output` global
    /// whose `global_remove` arrived.
    pub(super) outputs_removed: Vec<usize>,
    /// How many `wl_output` globals the registry has announced in all.
    pub(super) outputs_announced: usize,
}

/// Everything one [`Step::CaptureOn`] learned about its output.
#[derive(Clone, Debug, Default)]
pub(super) struct Capture {
    width: u32,
    height: u32,
    formats: Vec<u32>,
    dones: u32,
    stopped: bool,
    ready: bool,
    failed: bool,
    pixels: Vec<u8>,
}

/// A frame's outcome, from the client's side of the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Outcome {
    #[default]
    Waiting,
    Ready,
    Failed,
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    outputs: Vec<wl_output::WlOutput>,
    /// The registry name of each `wl_output` global, index-aligned with
    /// `outputs`, and those whose `global_remove` arrived.
    output_names: Vec<(u32, u32)>,
    outputs_removed: Vec<usize>,
    /// `closed` per layer surface, in creation order.
    layer_closed: Vec<bool>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    sources:
        Option<ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1>,
    capture: Option<ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1>,
    gamma_manager: Option<zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1>,
    /// The last configure size per layer surface, in creation order.
    layer_sizes: Vec<Option<(u32, u32)>>,
    /// `done` counts per requested frame callback, in request order.
    frame_dones: Vec<u32>,
    /// The newest constraint batch of the live capture session.
    constraints: Capture,
    incoming: Capture,
    frame: Outcome,
    /// `gamma_size` per held control, and whether it has seen `failed`.
    gamma_sizes: Vec<Option<u32>>,
    gamma_failed: Vec<bool>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
    fn event(
        client: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let (name, interface, version) = match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => (name, interface, version),
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = client
                    .output_names
                    .iter()
                    .position(|(known, _)| *known == name)
                {
                    client.outputs_removed.push(index);
                }
                return;
            }
            _ => return,
        };
        match interface.as_str() {
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(6), qh, ()));
            }
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "wl_output" => {
                client.output_names.push((name, version.min(4)));
                client
                    .outputs
                    .push(registry.bind(name, version.min(4), qh, ()));
            }
            "zwlr_layer_shell_v1" => {
                client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "ext_output_image_capture_source_manager_v1" => {
                client.sources = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "ext_image_copy_capture_manager_v1" => {
                client.capture = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "zwlr_gamma_control_manager_v1" => {
                client.gamma_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            _ => {}
        }
    }
}

/// Which layer surface a configure belongs to, in creation order.
struct SurfaceIndex(usize);

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, SurfaceIndex> for TestClient {
    fn event(
        client: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        index: &SurfaceIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                surface.ack_configure(serial);
                if let Some(slot) = client.layer_sizes.get_mut(index.0) {
                    *slot = Some((width, height));
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                if let Some(slot) = client.layer_closed.get_mut(index.0) {
                    *slot = true;
                }
            }
            _ => {}
        }
    }
}

/// Which frame callback a `done` belongs to, in request order.
struct FrameTag(usize);

impl Dispatch<wl_callback::WlCallback, FrameTag> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        tag: &FrameTag,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            if let Some(slot) = client.frame_dones.get_mut(tag.0) {
                *slot += 1;
            }
        }
    }
}

impl Dispatch<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                client.incoming.width = width;
                client.incoming.height = height;
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat { format } => {
                client.incoming.formats.push(format.into());
            }
            ext_image_copy_capture_session_v1::Event::Done => {
                client.incoming.dones += 1;
                client.constraints = std::mem::take(&mut client.incoming);
            }
            ext_image_copy_capture_session_v1::Event::Stopped => {
                client.constraints.stopped = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => client.frame = Outcome::Ready,
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => {
                client.frame = Outcome::Failed;
            }
            _ => {}
        }
    }
}

/// Which gamma control an event belongs to, in creation order.
struct ControlIndex(usize);

impl Dispatch<zwlr_gamma_control_v1::ZwlrGammaControlV1, ControlIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwlr_gamma_control_v1::ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        index: &ControlIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                if let Some(slot) = client.gamma_sizes.get_mut(index.0) {
                    *slot = Some(size);
                }
            }
            zwlr_gamma_control_v1::Event::Failed => {
                if let Some(slot) = client.gamma_failed.get_mut(index.0) {
                    *slot = true;
                }
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
wayland_client::delegate_noop!(TestClient: ignore ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1);
wayland_client::delegate_noop!(TestClient: ignore ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1);
wayland_client::delegate_noop!(TestClient: ignore ext_image_capture_source_v1::ExtImageCaptureSourceV1);
wayland_client::delegate_noop!(TestClient: ignore zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1);

/// A `width`x`height` `wl_buffer` filled with `color`, over a real memfd --
/// the same path any toolkit takes. Returns the pool file (for reading the
/// capture back), the buffer, and the pool length.
fn solid_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<TestClient>,
    width: i32,
    height: i32,
    color: [u8; 4],
) -> (std::fs::File, wl_buffer::WlBuffer, usize) {
    let stride = width * 4;
    let len = (stride * height) as usize;
    let fd = rustix::fs::memfd_create("scoot-per-output", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut file = std::fs::File::from(fd);
    let pixels: Vec<u8> = color.iter().copied().cycle().take(len).collect();
    file.write_all(&pixels).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    (file, buffer, len)
}

fn read_back(file: &std::fs::File, len: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; len];
    file.read_exact_at(&mut pixels, 0)
        .expect("the client's own pool is readable");
    pixels
}

type KeptBar = (
    wl_surface::WlSurface,
    zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
);
type KeptGamma = zwlr_gamma_control_v1::ZwlrGammaControlV1;

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    // Twice: the first round trip binds whatever globals the registry
    // announced, the second collects the events those binds produced.
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let shm = client.shm.clone().ok_or("no wl_shm")?;
    let layer_shell = client.layer_shell.clone().ok_or("no zwlr_layer_shell_v1")?;
    let sources = client.sources.clone().ok_or("no capture source manager")?;
    let capture = client.capture.clone().ok_or("no capture manager")?;
    let gamma_manager = client.gamma_manager.clone().ok_or("no gamma manager")?;

    let mut bars: Vec<KeptBar> = Vec::new();
    let mut frames: Vec<wl_callback::WlCallback> = Vec::new();
    let mut gammas: Vec<KeptGamma> = Vec::new();
    let mut session: Option<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1> = None;
    let mut frame: Option<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1> = None;
    let mut held: Option<(std::fs::File, wl_buffer::WlBuffer, usize)> = None;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let outcome = match step {
            Step::BarOn { output, color } => {
                let wl_output = client
                    .outputs
                    .get(output)
                    .cloned()
                    .ok_or_else(|| format!("no wl_output at index {output}"))?;
                let surface = compositor.create_surface(&qh, ());
                let index = client.layer_sizes.len();
                client.layer_sizes.push(None);
                client.layer_closed.push(false);
                let layer = layer_shell.get_layer_surface(
                    &surface,
                    Some(&wl_output),
                    zwlr_layer_shell_v1::Layer::Top,
                    "scoot-per-output".to_owned(),
                    &qh,
                    SurfaceIndex(index),
                );
                // A bar: full width, fixed height, anchored to three edges,
                // no exclusive zone so no window moves for it.
                layer.set_size(0, BAR_HEIGHT as u32);
                layer.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Top
                        | zwlr_layer_surface_v1::Anchor::Left
                        | zwlr_layer_surface_v1::Anchor::Right,
                );
                layer.set_exclusive_zone(0);
                surface.commit();
                // The initial configure carries the size to draw at -- the
                // compositor's choice, not this client's.
                let (width, height) =
                    wait_for(&mut queue, &mut client, "a layer configure", |client| {
                        client.layer_sizes[index]
                    })?;
                let (file, buffer, _) = solid_buffer(&shm, &qh, width as i32, height as i32, color);
                drop(file);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, width as i32, height as i32);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                bars.push((surface, layer));
                Ack::Bar(bars.len() - 1)
            }
            Step::RequestBarFrame { bar } => {
                let (surface, _) = bars.get(bar).ok_or("no such bar")?;
                let tag = FrameTag(client.frame_dones.len());
                client.frame_dones.push(0);
                // Kept alive: dropping the proxy is what turns a late `done`
                // into a dead connection.
                let callback = surface.frame(&qh, tag);
                surface.commit();
                frames.push(callback);
                // Flushed before answering, so the request is on the wire
                // when the test renders: without this the commit would sit
                // in the client buffer until the *next* step's round trip,
                // and a render in between would complete nothing.
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::BarFrames => {
                for _ in 0..5 {
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                }
                Ack::Frames(client.frame_dones.clone())
            }
            Step::CaptureOn { output } => {
                let captured = capture_on(
                    &mut queue,
                    &mut client,
                    &qh,
                    &shm,
                    &sources,
                    &capture,
                    &mut session,
                    &mut frame,
                    &mut held,
                    output,
                    true,
                )?;
                Ack::Captured(captured)
            }
            Step::CaptureNoWait { output } => {
                capture_on(
                    &mut queue,
                    &mut client,
                    &qh,
                    &shm,
                    &sources,
                    &capture,
                    &mut session,
                    &mut frame,
                    &mut held,
                    output,
                    false,
                )?;
                Ack::Done
            }
            Step::GammaOn { output } => {
                let wl_output = client
                    .outputs
                    .get(output)
                    .cloned()
                    .ok_or_else(|| format!("no wl_output at index {output}"))?;
                let index = client.gamma_sizes.len();
                client.gamma_sizes.push(None);
                client.gamma_failed.push(false);
                let control = gamma_manager.get_gamma_control(&wl_output, &qh, ControlIndex(index));
                wait_for(&mut queue, &mut client, "a gamma answer", |client| {
                    (client.gamma_sizes[index].is_some() || client.gamma_failed[index])
                        .then_some(())
                })?;
                let answer = Ack::Gamma {
                    size: client.gamma_sizes[index],
                    failed: client.gamma_failed[index],
                };
                control.destroy();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                answer
            }
            Step::GammaHold { output } => {
                let wl_output = client
                    .outputs
                    .get(output)
                    .cloned()
                    .ok_or_else(|| format!("no wl_output at index {output}"))?;
                let index = client.gamma_sizes.len();
                client.gamma_sizes.push(None);
                client.gamma_failed.push(false);
                let control = gamma_manager.get_gamma_control(&wl_output, &qh, ControlIndex(index));
                wait_for(&mut queue, &mut client, "a gamma answer", |client| {
                    (client.gamma_sizes[index].is_some() || client.gamma_failed[index])
                        .then_some(())
                })?;
                gammas.push(control);
                Ack::Held(index)
            }
            Step::Removals => {
                for _ in 0..3 {
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                }
                Ack::Removals(Removals {
                    bars_closed: client.layer_closed.clone(),
                    capture_stopped: client.constraints.stopped,
                    outputs_removed: client.outputs_removed.clone(),
                    outputs_announced: client.output_names.len(),
                })
            }
            Step::Rebind { output } => {
                let (name, version) = client
                    .output_names
                    .get(output)
                    .copied()
                    .ok_or_else(|| format!("no wl_output at index {output}"))?;
                let registry = registry.clone();
                let rebound: wl_output::WlOutput = registry.bind(name, version, &qh, ());
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                rebound.release();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
            Step::GammaState { held } => {
                for _ in 0..3 {
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                }
                Ack::GammaState {
                    size: client.gamma_sizes.get(held).copied().flatten(),
                    failed: client.gamma_failed.get(held).copied().unwrap_or(false),
                }
            }
        };
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One capture of `outputs[output]`: source, session, constraints, a frame
/// into a fresh buffer. With `wait`, waits for the outcome and reads the
/// buffer back; without, returns once the request is on the wire.
#[allow(clippy::too_many_arguments)]
fn capture_on(
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    qh: &QueueHandle<TestClient>,
    shm: &wl_shm::WlShm,
    sources: &ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    capture: &ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
    session: &mut Option<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1>,
    frame: &mut Option<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1>,
    held: &mut Option<(std::fs::File, wl_buffer::WlBuffer, usize)>,
    output: usize,
    wait: bool,
) -> Result<Capture, String> {
    let wl_output = client
        .outputs
        .get(output)
        .cloned()
        .ok_or_else(|| format!("no wl_output at index {output}"))?;
    if let Some(session) = session.take() {
        session.destroy();
    }
    if let Some(previous) = frame.take() {
        previous.destroy();
    }
    *held = None;
    client.frame = Outcome::Waiting;
    client.constraints = Capture::default();
    client.incoming = Capture::default();
    let source = sources.create_source(&wl_output, qh, ());
    let new_session = capture.create_session(
        &source,
        ext_image_copy_capture_manager_v1::Options::empty(),
        qh,
        (),
    );
    // The session outlives its source; destroying it here proves it.
    source.destroy();
    wait_for(queue, client, "a constraint batch", |client| {
        (client.constraints.dones > 0 || client.constraints.stopped).then_some(())
    })?;
    let mut captured = client.constraints.clone();
    *session = Some(new_session);
    if captured.stopped {
        return Ok(captured);
    }
    let (file, buffer, len) = solid_buffer(
        shm,
        qh,
        captured.width as i32,
        captured.height as i32,
        SENTINEL,
    );
    let live = session.as_ref().ok_or("no session")?;
    let new_frame = live.create_frame(qh, ());
    new_frame.attach_buffer(&buffer);
    new_frame.damage_buffer(0, 0, captured.width as i32, captured.height as i32);
    new_frame.capture();
    *frame = Some(new_frame);
    *held = Some((file, buffer, len));
    if !wait {
        return Ok(captured);
    }
    wait_for(queue, client, "a frame outcome", |client| {
        (client.frame != Outcome::Waiting).then_some(())
    })?;
    captured.ready = client.frame == Outcome::Ready;
    captured.failed = client.frame == Outcome::Failed;
    let (file, _, len) = held.as_ref().ok_or("no buffer")?;
    captured.pixels = read_back(file, *len);
    Ok(captured)
}

/// A live compositor with `count` outputs and one connected client.
///
/// The extra outputs are added before the client connects, so the registry
/// announces every one of them in the client's first round trip -- the same
/// order `compositor::run` builds them in.
pub(super) fn session(count: i32) -> Harness<Step, Ack> {
    let mut harness = Harness::headless(Appearance::default(), CANVAS);
    for index in 2..=count {
        headless::add_output(
            &mut harness.state,
            &format!("{OUTPUT_NAME}-{index}"),
            CANVAS,
            CANVAS,
        )
        .expect("another headless output");
    }
    harness.spawn(run_client);
    harness
}

/// Draws every output and hands back output `id`'s raw BGRA pixels.
pub(super) fn draw(harness: &mut Harness<Step, Ack>, id: OutputId) -> Vec<u8> {
    harness.state.request_render();
    harness.state.render();
    harness.pixels_of(id)
}

pub(super) fn bar_on(harness: &mut Harness<Step, Ack>, output: usize) -> usize {
    match harness.run(Step::BarOn {
        output,
        color: BAR_BGRA,
    }) {
        Ack::Bar(bar) => bar,
        _ => panic!("expected the bar index"),
    }
}

// ---------------------------------------------------------------------------
// The render loop: one strip per output
// ---------------------------------------------------------------------------

/// The bar mapped on the second output is in the second output's
/// framebuffer, and nowhere else.
///
/// The cross-wire pin: answering output 2 from output 1's framebuffer --
/// the refusal-to-serve rule this replaces -- would hand back output 1's
/// bare desktop here, and drawing nothing at all would hand back zeroes.
#[test]
fn each_output_composites_its_own_strip() {
    let mut harness = session(2);
    bar_on(&mut harness, 1);

    let first = draw(&mut harness, OutputId(1));
    let second = draw(&mut harness, OutputId(2));

    assert!(
        !contains(&first, BAR_BGRA),
        "the first output shows its own strip, not the second's bar"
    );
    assert!(
        contains(&second, BAR_BGRA),
        "the second output shows the bar mapped on it"
    );
    // The bar's rows, not just anywhere: full width at the top.
    for x in [0, CANVAS / 2, CANVAS - 1] {
        assert_eq!(
            pixel(&second, CANVAS, x, 0),
            BAR_BGRA,
            "the bar covers the second output's top row at x={x}"
        );
    }
}

/// A second output with nothing on it captures blank but correct: its own
/// background, not zeroes and not the first output's content.
#[test]
fn a_second_output_with_nothing_on_it_captures_blank_but_correct() {
    let mut harness = session(2);
    bar_on(&mut harness, 0);

    let first = draw(&mut harness, OutputId(1));
    let second = draw(&mut harness, OutputId(2));

    assert!(
        contains(&first, BAR_BGRA),
        "the control: the bar really drew on the first output"
    );
    assert!(
        !contains(&second, BAR_BGRA),
        "nothing mapped on the second output, so no bar colour may be there"
    );
    assert!(
        second.chunks_exact(4).all(|pixel| pixel == &second[0..4]),
        "a blank output is uniformly its background"
    );
    assert!(
        !second.iter().all(|byte| *byte == 0),
        "a blank output is background pixels, not an undrawn zero buffer"
    );
}

/// The pointer is drawn into the capture of the output it is on, at its
/// position *on that output*, and into no other output's capture -- and a
/// frame that composites the cursor (the `--tty` shape, forced here through
/// the frame seam) does the same.
#[test]
fn the_pointer_is_drawn_only_into_the_output_it_is_on() {
    let mut harness = session(2);
    let second = harness
        .state
        .outputs
        .get(OutputId(2))
        .cloned()
        .expect("a second output");
    let origin = harness
        .state
        .space
        .output_geometry(&second)
        .expect("the second output is placed")
        .loc;
    assert!(origin.x > 0 || origin.y > 0, "the outputs must not overlap");
    let shot = |harness: &mut Harness<Step, Ack>, id: OutputId| {
        harness
            .state
            .capture_pixels_for(Some(id), true)
            .expect("a capture")
            .bgra
    };

    // On output 1.
    harness.state.pointer_move(40.0, 40.0);
    let blank_first = draw(&mut harness, OutputId(1));
    let blank_second = harness.pixels_of(OutputId(2));
    let pointer_on_first = shot(&mut harness, OutputId(1));
    assert_ne!(pointer_on_first, blank_first, "output 1 shows the pointer");
    assert_eq!(
        shot(&mut harness, OutputId(2)),
        blank_second,
        "output 2 must not show a pointer that is on output 1"
    );

    // On output 2, at the same position on it.
    harness
        .state
        .pointer_move(f64::from(origin.x) + 40.0, f64::from(origin.y) + 40.0);
    assert_eq!(
        shot(&mut harness, OutputId(1)),
        blank_first,
        "the pointer left output 1's capture"
    );
    let pointer_on_second = shot(&mut harness, OutputId(2));
    assert_eq!(
        pointer_on_second, pointer_on_first,
        "at the same spot on an identical blank output: the same picture"
    );

    // Frames that composite the cursor place it the same way.
    harness.state.frame_cursor_for_test = Some(true);
    assert_eq!(draw(&mut harness, OutputId(1)), blank_first);
    assert_eq!(harness.pixels_of(OutputId(2)), pointer_on_second);
}

/// `screenshot` serves each output's own framebuffer: the IPC read-back of
/// output 2 is output 2's pixels, byte for byte.
#[test]
fn screenshot_serves_each_output_own_framebuffer() {
    let mut harness = session(2);
    bar_on(&mut harness, 1);
    harness.state.request_render();
    harness.state.render();

    let first = harness
        .state
        .capture_pixels_for(Some(OutputId(1)), false)
        .expect("a capture of the first output");
    let second = harness
        .state
        .capture_pixels_for(Some(OutputId(2)), false)
        .expect("a capture of the second output");
    assert_eq!((first.width, first.height), (CANVAS, CANVAS));
    assert_eq!((second.width, second.height), (CANVAS, CANVAS));
    assert!(
        !contains(&first.bgra, BAR_BGRA),
        "output 1's screenshot must not show output 2's bar"
    );
    assert!(
        contains(&second.bgra, BAR_BGRA),
        "output 2's screenshot must show its own bar"
    );
    assert_eq!(
        second.bgra,
        draw(&mut harness, OutputId(2)),
        "the screenshot is the framebuffer, not a second rendering of it"
    );
}

/// Eight outputs -- the `--outputs` maximum -- each render their own blank
/// strip without disturbing the others.
#[test]
fn eight_outputs_each_render() {
    let mut harness = session(8);
    assert_eq!(
        harness.state.backends.len(),
        8,
        "every output has a render target of its own"
    );
    harness.state.request_render();
    harness.state.render();
    for id in 1..=8 {
        let pixels = harness.pixels_of(OutputId(id));
        assert!(
            !pixels.iter().all(|byte| *byte == 0),
            "output {id} drew its background"
        );
    }
}

// ---------------------------------------------------------------------------
// Screen capture per output
// ---------------------------------------------------------------------------

/// `screencopy` of the second output is accepted and carries the second
/// output's pixels -- the refusal this replaces answered `stopped` here.
#[test]
fn screencopy_serves_the_source_output_own_pixels() {
    let mut harness = session(2);
    bar_on(&mut harness, 1);

    let first = match harness.run(Step::CaptureOn { output: 0 }) {
        Ack::Captured(captured) => captured,
        _ => panic!("expected a capture"),
    };
    assert!(!first.stopped, "the first output's source must be accepted");
    assert!(first.ready, "the first output's capture must succeed");
    assert!(
        !contains(&first.pixels, BAR_BGRA),
        "output 1's capture must not show output 2's bar"
    );

    let second = match harness.run(Step::CaptureOn { output: 1 }) {
        Ack::Captured(captured) => captured,
        _ => panic!("expected a capture"),
    };
    assert!(
        !second.stopped,
        "the second output's source must be accepted"
    );
    assert_eq!(
        (second.width, second.height),
        (CANVAS as u32, CANVAS as u32),
        "the constraints are the source output's own framebuffer size"
    );
    assert!(second.ready, "the second output's capture must succeed");
    assert!(
        contains(&second.pixels, BAR_BGRA),
        "output 2's capture must show its own bar"
    );
}

/// A capture of an output with nothing mapped succeeds and is blank.
#[test]
fn screencopy_of_an_empty_output_is_blank() {
    let mut harness = session(2);
    let second = match harness.run(Step::CaptureOn { output: 1 }) {
        Ack::Captured(captured) => captured,
        _ => panic!("expected a capture"),
    };
    assert!(!second.stopped);
    assert!(second.ready);
    assert!(
        !contains(&second.pixels, BAR_BGRA),
        "nothing mapped anywhere, so no bar colour may be captured"
    );
}

/// A client that goes away with a capture parked on the second output must
/// not wedge its session list or the next render.
#[test]
fn disconnect_mid_capture_on_the_second_output() {
    let mut harness = session(2);
    harness.run(Step::CaptureNoWait { output: 1 });
    harness.disconnect(0);
    harness.settle();
    assert_eq!(
        harness.state.screencopy.session_count(),
        (0, 0),
        "the dead session must be gone from both lists"
    );
    // ...and the outputs still draw afterwards.
    let second = draw(&mut harness, OutputId(2));
    assert!(
        !second.iter().all(|byte| *byte == 0),
        "the second output still renders after the disconnect"
    );
}

// ---------------------------------------------------------------------------
// Gamma per output
// ---------------------------------------------------------------------------

/// A gamma control for the second output gets its size, not `failed` -- the
/// refusal this replaces failed any non-primary output.
#[test]
fn gamma_is_per_output() {
    let mut harness = session(2);
    for output in 0..2 {
        match harness.run(Step::GammaOn { output }) {
            Ack::Gamma { size, failed } => assert_eq!(
                (size, failed),
                (Some(256), false),
                "output {output}'s control must get its size"
            ),
            _ => panic!("expected the gamma answer"),
        }
    }
}

/// Exclusivity transfers stay within their output: a second control on
/// output 1 fails the first control on output 1 and leaves output 2's
/// alone.
#[test]
fn gamma_transfers_stay_within_their_output() {
    let mut harness = session(2);
    let first = match harness.run(Step::GammaHold { output: 0 }) {
        Ack::Held(held) => held,
        _ => panic!("expected the held slot"),
    };
    let second = match harness.run(Step::GammaHold { output: 1 }) {
        Ack::Held(held) => held,
        _ => panic!("expected the held slot"),
    };
    let _third = match harness.run(Step::GammaHold { output: 0 }) {
        Ack::Held(held) => held,
        _ => panic!("expected the held slot"),
    };

    match harness.run(Step::GammaState { held: first }) {
        Ack::GammaState { failed, .. } => assert!(
            failed,
            "the superseded control on output 1 must have been failed"
        ),
        _ => panic!("expected the gamma state"),
    }
    match harness.run(Step::GammaState { held: second }) {
        Ack::GammaState { size, failed } => assert_eq!(
            (size, failed),
            (Some(256), false),
            "output 2's control must be untouched by output 1's transfer"
        ),
        _ => panic!("expected the gamma state"),
    }
    assert_eq!(
        harness.state.gamma_control.live_control_count(),
        2,
        "one live control per output"
    );
}

// ---------------------------------------------------------------------------
// Frame callbacks per output
// ---------------------------------------------------------------------------

/// Each output's frame completes its own surfaces' callbacks: a bar on each
/// output is woken by its own output's tick, at its own cadence.
#[test]
fn frame_callbacks_fire_per_output() {
    let mut harness = session(2);
    let first_bar = bar_on(&mut harness, 0);
    let second_bar = bar_on(&mut harness, 1);
    harness.run(Step::RequestBarFrame { bar: first_bar });
    harness.run(Step::RequestBarFrame { bar: second_bar });

    harness.state.request_render();
    harness.state.render();
    match harness.run(Step::BarFrames) {
        Ack::Frames(dones) => assert_eq!(
            dones,
            vec![1, 1],
            "each output's frame completes its own bar's callback"
        ),
        _ => panic!("expected the frame counts"),
    }

    // No new requests: a second frame completes nothing new on either.
    harness.state.request_render();
    harness.state.render();
    match harness.run(Step::BarFrames) {
        Ack::Frames(dones) => assert_eq!(
            dones,
            vec![1, 1],
            "callbacks fire once per request, on neither output twice"
        ),
        _ => panic!("expected the frame counts"),
    }
}
