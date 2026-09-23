//! Every advertised layout this suite can build, imported for real and
//! checked on screen.
//!
//! The parent suite's import tests build their buffers through
//! `/dev/udmabuf`, and a GLES renderer on Mesa's `kms_swrast` refuses a
//! udmabuf whatever its format (buffer *provenance*, not layout -- see
//! `test_support::test_renderer`). So under `SCOOT_TEST_RENDERER=gles` those
//! tests cannot say anything about which layouts import. This suite uses the
//! one provenance that driver does take: a **DRM dumb buffer** on
//! `/dev/dri/card0`, exported as a PRIME fd, i.e. a dma-buf the importing
//! device allocated itself. Multi-plane layouts are the same one fd named
//! once per plane at different offsets, which is exactly the shape a
//! decoder's `NV12` export has.
//!
//! What it pins, per layout, is the promise with teeth end to end: read the
//! **advertised** table off the wire, and for each `{fourcc, LINEAR}` in it
//! that [`LAYOUTS`] knows how to fill, allocate one, hand it over through
//! `create_immed` -- the request with no soft refusal, so a promise the
//! renderer breaks kills this client and fails the test -- show it in a real
//! `xdg_toplevel`, and assert the composited framebuffer shows the colour it
//! was filled with. Every layout is filled with the same opaque **red**, in
//! its own encoding (BT.601 limited-range `Y'CbCr` for the YUV ones), so one
//! pixel predicate covers them all and a layout sampled wrongly -- chroma
//! planes ignored, channels swapped, a YUV image bound as `GL_TEXTURE_2D` --
//! shows up as the wrong colour rather than passing.
//!
//! Where this machine has no usable `/dev/dri/card0` (CI, a GPU-less
//! container) the suite says so on stderr and asserts nothing, the same
//! trade the udmabuf suite makes (see [`super::skipped`]).

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::reexports::drm;
use smithay::reexports::drm::buffer::Buffer as _;
use smithay::reexports::drm::control::Device as _;
use smithay::reexports::drm::control::dumbbuffer::DumbBuffer;
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::read_format_table;
use crate::cli::RendererKind;
use crate::compositor::decorations::Appearance;
use crate::compositor::test_support::{Harness, wait_for};

/// The framebuffer the window is composited into.
const CANVAS: i32 = 128;

/// The side of every test buffer, in pixels. Even, so every chroma plane
/// here (2x2- and 2x1-subsampled) has whole samples.
const SIDE: u32 = 32;

/// How a buffer of one fourcc is laid out and filled with opaque red.
struct Layout {
    fourcc: Fourcc,
    /// Per plane: `(bytes per sample, horizontal subsampling, vertical
    /// subsampling)`. A plane's stride is `SIDE / h * bytes` and it has
    /// `SIDE / v` rows.
    planes: &'static [(u32, u32, u32)],
    /// Writes opaque red into `planes`, one byte slice per plane in order.
    fill: fn(&mut [&mut [u8]]),
}

/// BT.601 limited-range red: `Y' = 81`, `Cb = 90`, `Cr = 240`. Mesa's EGL
/// default for an imported YUV image is BT.601 narrow range, so this lands
/// within a few units of `(255, 0, 0)`.
const RED_Y: u8 = 81;
const RED_U: u8 = 90;
const RED_V: u8 = 240;

/// One representative per layout *class*, not one per fourcc: the two
/// candidates; another 8-bit channel order; 10-bit packed; one- and
/// two-channel; half-float; and the four YUV shapes a video pipeline hands
/// over -- two-plane 8-bit (`NV12`), two-plane 16-bit (`P010`), three-plane
/// (`YU12`) and packed 4:2:2 (`YUYV`). An advertised fourcc outside this
/// list is reported, not tested (see [`every_advertised_layout_imports_and_draws`]).
const LAYOUTS: &[Layout] = &[
    Layout {
        fourcc: Fourcc::Xrgb8888,
        planes: &[(4, 1, 1)],
        fill: |planes| fill_repeating(planes, 0, &[0x00, 0x00, 0xFF, 0x00]),
    },
    Layout {
        fourcc: Fourcc::Argb8888,
        planes: &[(4, 1, 1)],
        fill: |planes| fill_repeating(planes, 0, &[0x00, 0x00, 0xFF, 0xFF]),
    },
    Layout {
        fourcc: Fourcc::Abgr8888,
        planes: &[(4, 1, 1)],
        fill: |planes| fill_repeating(planes, 0, &[0xFF, 0x00, 0x00, 0xFF]),
    },
    Layout {
        // A:R:G:B 2:10:10:10, little-endian: red is 0x3FF << 20, alpha 3 << 30.
        fourcc: Fourcc::Argb2101010,
        planes: &[(4, 1, 1)],
        fill: |planes| fill_repeating(planes, 0, &0xFFF0_0000u32.to_le_bytes()),
    },
    Layout {
        // One channel, sampled as `(r, 0, 0, 1)`.
        fourcc: Fourcc::R8,
        planes: &[(1, 1, 1)],
        fill: |planes| fill_repeating(planes, 0, &[0xFF]),
    },
    Layout {
        // G:R 8:8 little-endian: the first byte is red.
        fourcc: Fourcc::Gr88,
        planes: &[(2, 1, 1)],
        fill: |planes| fill_repeating(planes, 0, &[0xFF, 0x00]),
    },
    Layout {
        // A:B:G:R 16:16:16:16 half-float, little-endian: R, G, B, A in memory
        // order; 1.0 is 0x3C00.
        fourcc: Fourcc::Abgr16161616f,
        planes: &[(8, 1, 1)],
        fill: |planes| {
            fill_repeating(planes, 0, &[0x00, 0x3C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3C]);
        },
    },
    Layout {
        fourcc: Fourcc::Nv12,
        planes: &[(1, 1, 1), (2, 2, 2)],
        fill: |planes| {
            fill_repeating(planes, 0, &[RED_Y]);
            fill_repeating(planes, 1, &[RED_U, RED_V]);
        },
    },
    Layout {
        // 16-bit little-endian samples with the value in the top 10 bits:
        // the 8-bit value lands in the high byte.
        fourcc: Fourcc::P010,
        planes: &[(2, 1, 1), (4, 2, 2)],
        fill: |planes| {
            fill_repeating(planes, 0, &[0x00, RED_Y]);
            fill_repeating(planes, 1, &[0x00, RED_U, 0x00, RED_V]);
        },
    },
    Layout {
        // I420: Y, then Cb, then Cr, each chroma plane 2x2-subsampled.
        fourcc: Fourcc::Yuv420,
        planes: &[(1, 1, 1), (1, 2, 2), (1, 2, 2)],
        fill: |planes| {
            fill_repeating(planes, 0, &[RED_Y]);
            fill_repeating(planes, 1, &[RED_U]);
            fill_repeating(planes, 2, &[RED_V]);
        },
    },
    Layout {
        // Packed 4:2:2: Y0 Cb Y1 Cr per two pixels, i.e. two bytes a pixel.
        fourcc: Fourcc::Yuyv,
        planes: &[(2, 1, 1)],
        fill: |planes| fill_repeating(planes, 0, &[RED_Y, RED_U, RED_Y, RED_V]),
    },
];

fn fill_repeating(planes: &mut [&mut [u8]], index: usize, pattern: &[u8]) {
    for (byte, value) in planes[index].iter_mut().zip(pattern.iter().cycle()) {
        *byte = *value;
    }
}

/// One instruction for the client thread.
enum Step {
    /// Bind the dmabuf global at v4, read the default feedback's table.
    ReadTable,
    /// Allocate, fill and `create_immed` one buffer of `LAYOUTS[index]`, and
    /// show it in the client's one window (mapping it on first use).
    Show { index: usize },
}

/// What a client answers a [`Step`] with.
enum Ack {
    Table(Vec<(u32, u64)>),
    Shown,
    /// This machine has no usable `/dev/dri/card0` to allocate from.
    NoDevice(String),
}

type Fixture = Harness<Step, Ack>;

/// `/dev/dri/card0`, as the `drm` crate's device traits want it.
struct Card(std::fs::File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl drm::Device for Card {}
impl drm::control::Device for Card {}

/// A filled dumb buffer and the PRIME fd a client hands over for it. The
/// card is kept open for as long as the buffer is: the GEM handle belongs
/// to that open file.
struct Allocation {
    card: Card,
    dumb: Option<DumbBuffer>,
    fd: OwnedFd,
    /// `(offset, stride)` per plane, in plane order.
    planes: Vec<(u32, u32)>,
}

impl Drop for Allocation {
    fn drop(&mut self) {
        if let Some(dumb) = self.dumb.take() {
            let _ = self.card.destroy_dumb_buffer(dumb);
        }
    }
}

/// Allocates `layout` as one dumb buffer on `/dev/dri/card0`, filled with
/// red, planes packed back to back. `Err` names why this machine cannot.
fn allocate(layout: &Layout) -> Result<Allocation, String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/dri/card0")
        .map_err(|error| format!("/dev/dri/card0: {error}"))?;
    let card = Card(file);
    let mut planes = Vec::with_capacity(layout.planes.len());
    let mut total = 0u32;
    for &(bytes, h, v) in layout.planes {
        let stride = SIDE / h * bytes;
        planes.push((total, stride));
        total += stride * (SIDE / v);
    }
    // A 32bpp dumb buffer 64 pixels (256 bytes) wide, tall enough for every
    // plane: its own format and pitch are irrelevant, only its bytes are
    // named by the offsets and strides above.
    let rows = total.div_ceil(256);
    let mut dumb = card
        .create_dumb_buffer((64, rows), drm::buffer::DrmFourcc::Xrgb8888, 32)
        .map_err(|error| format!("DRM_IOCTL_MODE_CREATE_DUMB: {error}"))?;
    {
        let mut mapping = card
            .map_dumb_buffer(&mut dumb)
            .map_err(|error| format!("DRM_IOCTL_MODE_MAP_DUMB: {error}"))?;
        // The planes are back to back from offset 0, so splitting off each
        // one's length in turn is exactly its range.
        let mut rest: &mut [u8] = &mut mapping;
        let mut slices: Vec<&mut [u8]> = Vec::with_capacity(planes.len());
        for (&(_, stride), &(_, _, v)) in planes.iter().zip(layout.planes) {
            let (plane, tail) =
                std::mem::take(&mut rest).split_at_mut((stride * (SIDE / v)) as usize);
            slices.push(plane);
            rest = tail;
        }
        (layout.fill)(&mut slices);
    }
    let fd = card
        .buffer_to_prime_fd(dumb.handle(), drm::CLOEXEC | drm::RDWR)
        .map_err(|error| format!("DRM_IOCTL_PRIME_HANDLE_TO_FD: {error}"))?;
    Ok(Allocation {
        card,
        dumb: Some(dumb),
        fd,
        planes,
    })
}

#[derive(Default)]
struct TestClient {
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    table: Option<Vec<(u32, u64)>>,
    window: Option<(
        wl_surface::WlSurface,
        xdg_surface::XdgSurface,
        xdg_toplevel::XdgToplevel,
    )>,
    window_serial: Option<u32>,
    /// The buffer on screen and the allocation behind it, kept until the
    /// next one replaces them.
    shown: Option<(wl_buffer::WlBuffer, Allocation)>,
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let ack = match step {
            Step::ReadTable => {
                let dmabuf = client.dmabuf.clone().ok_or("no zwp_linux_dmabuf_v1")?;
                client.table = None;
                let feedback = dmabuf.get_default_feedback(&qh, ());
                let table = wait_for(&mut queue, &mut client, "default feedback", |client| {
                    client.table.clone()
                })?;
                feedback.destroy();
                Ack::Table(table)
            }
            Step::Show { index } => {
                let layout = &LAYOUTS[index];
                match allocate(layout) {
                    Err(reason) => Ack::NoDevice(reason),
                    Ok(allocation) => {
                        show(&mut client, &mut queue, &qh, layout, allocation)?;
                        Ack::Shown
                    }
                }
            }
        };
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `create_immed` a buffer over `allocation` and put it on screen.
fn show(
    client: &mut TestClient,
    queue: &mut wayland_client::EventQueue<TestClient>,
    qh: &QueueHandle<TestClient>,
    layout: &Layout,
    allocation: Allocation,
) -> Result<(), String> {
    let dmabuf = client.dmabuf.clone().ok_or("no zwp_linux_dmabuf_v1")?;
    let params = dmabuf.create_params(qh, ());
    let linear = u64::from(Modifier::Linear);
    for (index, &(offset, stride)) in allocation.planes.iter().enumerate() {
        params.add(
            allocation.fd.as_fd(),
            index as u32,
            offset,
            stride,
            (linear >> 32) as u32,
            linear as u32,
        );
    }
    let buffer = params.create_immed(
        SIDE as i32,
        SIDE as i32,
        layout.fourcc as u32,
        zwp_linux_buffer_params_v1::Flags::empty(),
        qh,
        (),
    );
    params.destroy();

    if client.window.is_none() {
        let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
        let wm_base = client.wm_base.clone().ok_or("no xdg_wm_base")?;
        let surface = compositor.create_surface(qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, qh, ());
        let toplevel = xdg.get_toplevel(qh, ());
        surface.commit();
        let serial = wait_for(queue, client, "an xdg configure", |client| {
            client.window_serial
        })?;
        xdg.ack_configure(serial);
        client.window = Some((surface, xdg, toplevel));
    }
    let (surface, _, _) = client.window.as_ref().ok_or("no window")?;
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, SIDE as i32, SIDE as i32);
    surface.commit();
    // A refused `create_immed` is a fatal protocol error, which this round
    // trip surfaces as the client's own dispatch failure -- the test then
    // fails naming the layout, which is the point.
    queue
        .roundtrip(client)
        .map_err(|error| format!("{:?} import killed the client: {error}", layout.fourcc))?;
    if let Some((old, _allocation)) = client.shown.replace((buffer, allocation)) {
        old.destroy();
    }
    Ok(())
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
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        if interface == wl_compositor::WlCompositor::interface().name {
            client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
        } else if interface == xdg_wm_base::XdgWmBase::interface().name {
            client.wm_base = Some(registry.bind(name, 1, qh, ()));
        } else if interface == zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1::interface().name {
            // v4: Mesa's own bind version, and the first with feedback.
            client.dmabuf = Some(registry.bind(name, version.min(4), qh, ()));
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for TestClient {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            client.window_serial = Some(serial);
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_linux_dmabuf_feedback_v1::Event::FormatTable { fd, size } = event {
            client.table = Some(read_format_table(fd, size));
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(TestClient: ignore zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1);
wayland_client::delegate_noop!(TestClient: ignore zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1);

/// How many framebuffer pixels are unmistakably red. The framebuffer is BGRA.
/// Wide bands on purpose: a YUV conversion lands within a few units of pure
/// red, while the wrong colour -- chroma ignored (grey), channels swapped
/// (blue), a YUV image sampled as RGB (dark green) -- lands nowhere near.
fn red_pixels(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|pixel| pixel[2] >= 200 && pixel[1] <= 60 && pixel[0] <= 60)
        .count()
}

#[test]
fn every_advertised_layout_imports_and_draws() {
    let mut fixture = Harness::headless(Appearance::default(), CANVAS);
    fixture.spawn(run_client);
    let Ack::Table(table) = fixture.run(Step::ReadTable) else {
        panic!("expected the advertised table");
    };
    let linear = u64::from(Modifier::Linear);
    let mut drawn = Vec::new();
    for (index, layout) in LAYOUTS.iter().enumerate() {
        if !table.contains(&(layout.fourcc as u32, linear)) {
            continue;
        }
        match fixture.run(Step::Show { index }) {
            Ack::NoDevice(reason) => {
                eprintln!(
                    "every_advertised_layout_imports_and_draws: skipped -- no dumb \
                     buffer on this machine ({reason})"
                );
                return;
            }
            Ack::Shown => {}
            Ack::Table(_) => panic!("expected a shown buffer"),
        }
        let red = red_pixels(&fixture.render());
        let area = (SIDE * SIDE) as usize;
        assert!(
            red >= area / 2,
            "{:?} (advertised at LINEAR) imported but drew {red} red pixels of a \
             {SIDE}x{SIDE} red buffer -- sampled as the wrong colour, or not at all",
            layout.fourcc
        );
        drawn.push(layout.fourcc);
    }
    let untested: Vec<String> = table
        .iter()
        .filter(|(code, _)| !LAYOUTS.iter().any(|layout| layout.fourcc as u32 == *code))
        .map(|(code, modifier)| format!("{:?}/{modifier:#x}", Fourcc::try_from(*code)))
        .collect();
    eprintln!(
        "every_advertised_layout_imports_and_draws: drew {drawn:?}; advertised but \
         not built by this suite: {untested:?}"
    );
    // The candidates are advertised under every renderer this project has, so
    // a run that drew neither checked nothing and must not read as a pass.
    assert!(
        drawn.contains(&Fourcc::Xrgb8888) && drawn.contains(&Fourcc::Argb8888),
        "both candidates are advertised on every renderer here, so both must \
         have been drawn: {drawn:?}"
    );
    if fixture_renderer(&fixture) == RendererKind::Gles {
        // The multi-plane claim this suite exists for. A GLES renderer that
        // advertised none of them would make the loop above vacuous for the
        // layouts that matter most, so say so rather than pass.
        assert!(
            drawn.contains(&Fourcc::Nv12),
            "the GLES table is expected to carry NV12 on a Mesa driver; it \
             drew {drawn:?} from {} advertised pairs",
            table.len()
        );
    }
}

/// Which renderer the fixture really built (see `Fixture::renderer` in the
/// parent suite for why that is not `State::renderer`).
fn fixture_renderer(fixture: &Fixture) -> RendererKind {
    let id = fixture
        .state
        .outputs
        .primary_id()
        .expect("an output behind the fixture");
    fixture
        .state
        .backends
        .get(&id)
        .expect("a headless backend behind the fixture")
        .renderer()
}
