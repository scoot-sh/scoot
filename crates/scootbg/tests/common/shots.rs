//! Screenshots, to check real pixels.
//!
//! - scoot: its IPC `screenshot` request (what `scoot msg screenshot`
//!   sends), a base64 PNG of one output's framebuffer.
//! - sway (or any wlroots compositor): a minimal `wlr-screencopy-v1` client
//!   written here, so the tests need no screenshot tool on the machine. The
//!   frame is copied into a memfd and read back with plain `read`, so no
//!   mapping (and no `unsafe`) is needed.

use std::io::{Read, Seek};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;

use base64::Engine;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{
    self, ZwlrScreencopyFrameV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use super::{PATIENCE, Session};

/// An RGB image.
pub struct Shot {
    pub width: u32,
    pub height: u32,
    rgb: Vec<[u8; 3]>,
}

impl Shot {
    pub fn at(&self, x: u32, y: u32) -> [u8; 3] {
        assert!(
            x < self.width && y < self.height,
            "({x},{y}) outside the shot"
        );
        self.rgb[(y * self.width + x) as usize]
    }

    /// The distinct colors in the whole image, sorted.
    pub fn colors(&self) -> Vec<[u8; 3]> {
        let mut colors = self.rgb.clone();
        colors.sort_unstable();
        colors.dedup();
        colors
    }

    /// The centre, the four corners and the four edge midpoints, named.
    pub fn samples(&self) -> Vec<(&'static str, [u8; 3])> {
        let (w, h) = (self.width - 1, self.height - 1);
        [
            ("centre", w / 2, h / 2),
            ("top-left", 0, 0),
            ("top-right", w, 0),
            ("bottom-left", 0, h),
            ("bottom-right", w, h),
            ("top", w / 2, 0),
            ("bottom", w / 2, h),
            ("left", 0, h / 2),
            ("right", w, h / 2),
        ]
        .into_iter()
        .map(|(name, x, y)| (name, self.at(x, y)))
        .collect()
    }

    /// Asserts every pixel is `rgb`, naming the sampled points if not.
    pub fn assert_all(&self, rgb: [u8; 3], what: &str) {
        let colors = self.colors();
        assert_eq!(
            colors,
            [rgb],
            "{what}: not one flat color; samples {:?}",
            self.samples()
        );
    }
}

/// `#rrggbb` as RGB.
pub fn rgb(hex: &str) -> [u8; 3] {
    let hex = hex.strip_prefix('#').unwrap();
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
    [byte(0), byte(2), byte(4)]
}

impl Session {
    /// scoot output `id`'s pixels (1 is the first), without the pointer.
    pub fn scoot_screenshot(&self, id: u64) -> Shot {
        let reply = self.scoot_ipc(&format!(
            r#"{{"type":"screenshot","output":{id},"cursor":false}}"#
        ));
        assert_eq!(reply["type"], "screenshot", "{reply}");
        let png = base64::engine::general_purpose::STANDARD
            .decode(reply["png"].as_str().unwrap())
            .unwrap();
        decode_png(&png)
    }

    /// The pixels of the output named `name`, through wlr-screencopy (a
    /// sway session).
    pub fn screencopy(&self, name: &str) -> Shot {
        screencopy(&self.runtime_dir().join(&self.wayland_display), name)
    }
}

fn decode_png(bytes: &[u8]) -> Shot {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("unexpected PNG color type {other:?}"),
    };
    let rgb = buf[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    Shot {
        width: info.width,
        height: info.height,
        rgb,
    }
}

#[derive(Default)]
struct Copy {
    outputs: Vec<(WlOutput, Option<String>)>,
    /// `buffer` event: format, width, height, stride.
    offer: Option<(WEnum<wl_shm::Format>, u32, u32, u32)>,
    y_invert: bool,
    done: Option<Result<(), ()>>,
}

fn screencopy(socket: &Path, name: &str) -> Shot {
    let stream = UnixStream::connect(socket).unwrap();
    let conn = Connection::from_socket(stream).unwrap();
    let (globals, mut queue) = registry_queue_init::<Copy>(&conn).unwrap();
    let qh = queue.handle();
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
    let manager: ZwlrScreencopyManagerV1 = globals
        .bind(&qh, 1..=3, ())
        .expect("the compositor has no zwlr_screencopy_manager_v1");
    let mut state = Copy::default();
    globals.contents().with_list(|list| {
        for global in list.iter().filter(|g| g.interface == "wl_output") {
            let output: WlOutput =
                globals
                    .registry()
                    .bind(global.name, global.version.min(4), &qh, ());
            state.outputs.push((output, None));
        }
    });
    queue.roundtrip(&mut state).unwrap();
    let output = state
        .outputs
        .iter()
        .find(|(_, n)| n.as_deref() == Some(name))
        .map(|(o, _)| o.clone())
        .unwrap_or_else(|| panic!("no output named {name}"));
    let frame = manager.capture_output(0, &output, &qh, ());
    // The buffer offers are sent at once, before the round trip's reply.
    queue.roundtrip(&mut state).unwrap();
    let (format, width, height, stride) = state.offer.expect("no buffer offer");
    let len = stride * height;
    let fd = rustix::fs::memfd_create(c"shot", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
    rustix::fs::ftruncate(&fd, u64::from(len)).unwrap();
    let pool = shm.create_pool(fd.as_fd(), len as i32, &qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        format.into_result().unwrap(),
        &qh,
        (),
    );
    frame.copy(&buffer);
    let deadline = std::time::Instant::now() + PATIENCE;
    while state.done.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "screencopy never finished"
        );
        queue.blocking_dispatch(&mut state).unwrap();
    }
    assert_eq!(state.done, Some(Ok(())), "screencopy failed");
    let mut file = std::fs::File::from(fd);
    file.rewind().unwrap();
    let mut bytes = Vec::with_capacity(len as usize);
    file.read_to_end(&mut bytes).unwrap();
    // Little-endian 32-bit formats: XRGB/ARGB sit as B,G,R,X in memory,
    // XBGR/ABGR as R,G,B,X.
    let swap = match format.into_result().unwrap() {
        wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 => true,
        wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => false,
        other => panic!("unexpected screencopy format {other:?}"),
    };
    let mut rgb = Vec::with_capacity((width * height) as usize);
    for row in 0..height {
        let row = if state.y_invert {
            height - 1 - row
        } else {
            row
        };
        let start = (row * stride) as usize;
        for p in bytes[start..start + (width * 4) as usize].chunks_exact(4) {
            rgb.push(if swap {
                [p[2], p[1], p[0]]
            } else {
                [p[0], p[1], p[2]]
            });
        }
    }
    frame.destroy();
    buffer.destroy();
    pool.destroy();
    Shot { width, height, rgb }
}

impl Dispatch<WlRegistry, GlobalListContents> for Copy {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, ()> for Copy {
    fn event(
        state: &mut Self,
        output: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(entry) = state.outputs.iter_mut().find(|(o, _)| o == output) {
                entry.1 = Some(name);
            }
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for Copy {
    fn event(
        state: &mut Self,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                // The first shm offer.
                state.offer.get_or_insert((format, width, height, stride));
            }
            zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                state.y_invert = flags
                    .into_result()
                    .is_ok_and(|f| f.contains(zwlr_screencopy_frame_v1::Flags::YInvert));
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.done = Some(Ok(())),
            zwlr_screencopy_frame_v1::Event::Failed => state.done = Some(Err(())),
            _ => {}
        }
    }
}

delegate_noop!(Copy: ignore WlShm);
delegate_noop!(Copy: WlShmPool);
delegate_noop!(Copy: ignore WlBuffer);
delegate_noop!(Copy: ZwlrScreencopyManagerV1);
