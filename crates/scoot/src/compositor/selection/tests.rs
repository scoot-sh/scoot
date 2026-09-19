//! Tests for the clipboard and primary-selection globals.
//!
//! These drive *real* `wayland-client` connections -- binding the managers
//! and offering/setting/receiving selections exactly as `cliphist` or a
//! toolkit does -- through a real [`State`] with a real headless backend.
//! What is under test is what the *other* device observes (offers,
//! selections, bytes), which is invisible to a test that calls the handler
//! directly.
//!
//! Like `layer_shell/tests.rs`, the client runs on its own thread while the
//! test pumps the compositor, and each test is one client script run to
//! completion. Unlike that harness there is no step scripting: every test
//! here is linear, so the client thread runs a single closure and reports
//! back one result.
//!
//! These need a writable `$XDG_RUNTIME_DIR` for the same reason the other
//! compositor tests do ([`State::new`] binds a real listening socket either
//! way).

use std::io::{Read, Seek, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use scoot_core::Config;
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1, ext_data_control_manager_v1, ext_data_control_offer_v1,
    ext_data_control_source_v1,
};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_offer_v1, zwp_primary_selection_source_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
    zwlr_data_control_source_v1,
};

use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::ClientState;

/// What the tests pass around as clipboard content. Distinctive on purpose:
/// reading back exactly this is what proves the bytes went through the
/// compositor rather than anywhere else.
const PAYLOAD: &[u8] = b"scoot-clipboard-42";
const MIME: &str = "text/plain";

/// How long a client script may take before the compositor counts as not
/// answering. Generous: a debug build on a VM.
const PATIENCE: Duration = Duration::from_secs(20);

/// A live compositor with a real headless backend, serving client threads
/// that each run one script to completion.
struct Harness {
    event_loop: EventLoop<'static, State>,
    state: State,
}

impl Harness {
    fn new() -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, 200, 200).expect("a headless backend");
        Self { event_loop, state }
    }

    /// Connects one client over a socket pair (no dependence on the real
    /// listening socket's name) and runs `script` on its thread.
    fn run_client(
        &mut self,
        script: impl FnOnce(ClientConn) -> Result<String, String> + Send + 'static,
    ) -> JoinHandle<Result<String, String>> {
        let (server_end, client_end) = UnixStream::pair().expect("a socket pair");
        self.state
            .display_handle
            .insert_client(server_end, Arc::new(ClientState::default()))
            .expect("an inserted client");
        std::thread::spawn(move || {
            let conn = ClientConn::new(client_end)?;
            script(conn)
        })
    }

    /// Pumps the compositor until the client thread reports back, failing the
    /// test if the compositor stops serving first. A client that died instead
    /// of answering is reported with *its own* error (the protocol error it
    /// provoked, usually), not as a timeout.
    fn wait_for(&mut self, handle: JoinHandle<Result<String, String>>) -> Result<String, String> {
        let deadline = Instant::now() + PATIENCE;
        // The client thread owns its result; poll for its exit while pumping.
        loop {
            if handle.is_finished() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the client; the compositor stopped serving"
            );
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("a compositor dispatch");
        }
        handle.join().expect("the client thread")
    }
}

/// The client end of one connection: the socket, queue and dispatch state.
struct ClientConn {
    queue: wayland_client::EventQueue<Client>,
    client: Client,
}

impl ClientConn {
    fn new(stream: UnixStream) -> Result<Self, String> {
        let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let mut client = Client::default();
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        Ok(Self { queue, client })
    }

    fn roundtrip(&mut self) -> Result<(), String> {
        self.queue
            .roundtrip(&mut self.client)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Everything the selection clients bind or observe.
#[derive(Default)]
struct Client {
    /// Every `(interface, version)` the registry advertised, in order. The
    /// advertisement test reads this; the rest ignore it.
    globals: Vec<(String, u32)>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    keyboard_focus: Option<wl_surface::WlSurface>,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    wlr_manager: Option<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1>,
    ext_manager: Option<ext_data_control_manager_v1::ExtDataControlManagerV1>,
    primary_manager:
        Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    /// `(mime, offer)` pairs the observing device was offered, in order --
    /// only that device's, not the setter's echo (see below).
    wlr_offers: Vec<(
        Vec<String>,
        zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
    )>,
    wlr_selection: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,
    /// The device whose offers and selection count. A `set_selection` is
    /// broadcast to *every* data-control device of the seat, including the
    /// setter's own -- so without this gate the setter's echo would read as
    /// a second, phantom clipboard.
    wlr_observer: Option<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1>,
    ext_offers: Vec<(
        Vec<String>,
        ext_data_control_offer_v1::ExtDataControlOfferV1,
    )>,
    ext_selection: Option<ext_data_control_offer_v1::ExtDataControlOfferV1>,
    /// Same echo gate as `wlr_observer`, for the ext generation.
    ext_observer: Option<ext_data_control_device_v1::ExtDataControlDeviceV1>,
    primary_offers: Vec<(
        Vec<String>,
        zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
    )>,
    primary_selection: Option<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1>,
    /// Same echo gate as `wlr_observer`, for the primary selection: the
    /// setter hears its own selection echoed back too.
    primary_observer: Option<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1>,
    xdg_serial: Option<u32>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Client {
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
        client.globals.push((interface.clone(), version));
        match interface.as_str() {
            "wl_seat" => client.seat = Some(registry.bind(name, version.min(7), qh, ())),
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
            "xdg_wm_base" => client.wm_base = Some(registry.bind(name, version.min(3), qh, ())),
            "zwlr_data_control_manager_v1" => {
                client.wlr_manager = Some(registry.bind(name, version.min(2), qh, ()));
            }
            "ext_data_control_manager_v1" => {
                client.ext_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            "zwp_primary_selection_device_manager_v1" => {
                client.primary_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Client {
    fn event(
        client: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { .. } = event
            && client.keyboard.is_none()
        {
            client.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Enter { surface, .. } = event {
            client.keyboard_focus = Some(surface);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
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

impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        client: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            client.xdg_serial = Some(serial);
        }
    }
}

impl Dispatch<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
        _: zwlr_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_data_control_source_v1::ZwlrDataControlSourceV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_source_v1::Event::Send { fd, .. } = event {
            let mut file = std::fs::File::from(fd);
            file.write_all(PAYLOAD).expect("the send fd accepts bytes");
        }
    }
}

impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for Client {
    fn event(
        client: &mut Self,
        device: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Only the observer's events count -- see `wlr_observer`. The setter
        // hears its own selection echoed back, which is correct compositor
        // behavior and not a second clipboard.
        let observed = client.wlr_observer.as_ref().is_some_and(|o| o == device);
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id: offer } => {
                if observed {
                    client.wlr_offers.push((Vec::new(), offer));
                }
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                if observed {
                    client.wlr_selection = id;
                }
            }
            zwlr_data_control_device_v1::Event::Finished => {}
            zwlr_data_control_device_v1::Event::PrimarySelection { .. } => {}
            _ => {}
        }
    }

    /// The `data_offer` event carries the offer as a server-created `new_id`,
    /// so the client has to say what user data it gets. `()` here: with one
    /// offer in flight these tests file mime events under the latest offer
    /// (see below), which is exactly where they belong -- the offer object
    /// arrives before any of its mime types.
    fn event_created_child(
        opcode: u16,
        qh: &QueueHandle<Self>,
    ) -> std::sync::Arc<dyn wayland_client::backend::ObjectData> {
        assert_eq!(
            opcode, 0,
            "the only child-creating event here is data_offer"
        );
        qh.make_data::<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()>(())
    }
}

impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()> for Client {
    fn event(
        client: &mut Self,
        _: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event
            && let Some((mimes, _)) = client.wlr_offers.last_mut()
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<ext_data_control_manager_v1::ExtDataControlManagerV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &ext_data_control_manager_v1::ExtDataControlManagerV1,
        _: ext_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_data_control_source_v1::ExtDataControlSourceV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &ext_data_control_source_v1::ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_source_v1::Event::Send { fd, .. } = event {
            let mut file = std::fs::File::from(fd);
            file.write_all(PAYLOAD).expect("the send fd accepts bytes");
        }
    }
}

impl Dispatch<ext_data_control_device_v1::ExtDataControlDeviceV1, ()> for Client {
    fn event(
        client: &mut Self,
        device: &ext_data_control_device_v1::ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Same echo gate as the wlr device -- see `ext_observer`.
        let observed = client.ext_observer.as_ref().is_some_and(|o| o == device);
        match event {
            ext_data_control_device_v1::Event::DataOffer { id: offer } => {
                if observed {
                    client.ext_offers.push((Vec::new(), offer));
                }
            }
            ext_data_control_device_v1::Event::Selection { id } => {
                if observed {
                    client.ext_selection = id;
                }
            }
            ext_data_control_device_v1::Event::Finished => {}
            ext_data_control_device_v1::Event::PrimarySelection { .. } => {}
            _ => {}
        }
    }

    /// Same `new_id` story as the wlr device above: `data_offer` (opcode 0)
    /// is the only child-creating event.
    fn event_created_child(
        opcode: u16,
        qh: &QueueHandle<Self>,
    ) -> std::sync::Arc<dyn wayland_client::backend::ObjectData> {
        assert_eq!(
            opcode, 0,
            "the only child-creating event here is data_offer"
        );
        qh.make_data::<ext_data_control_offer_v1::ExtDataControlOfferV1, ()>(())
    }
}

impl Dispatch<ext_data_control_offer_v1::ExtDataControlOfferV1, ()> for Client {
    fn event(
        client: &mut Self,
        _: &ext_data_control_offer_v1::ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event
            && let Some((mimes, _)) = client.ext_offers.last_mut()
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1, ()>
    for Client
{
    fn event(
        _: &mut Self,
        _: &zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
        _: zwp_primary_selection_device_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_source_v1::Event::Send { fd, .. } = event {
            let mut file = std::fs::File::from(fd);
            file.write_all(PAYLOAD).expect("the send fd accepts bytes");
        }
    }
}

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()> for Client {
    fn event(
        client: &mut Self,
        device: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Same echo gate as the data-control devices.
        let observed = client
            .primary_observer
            .as_ref()
            .is_some_and(|o| o == device);
        match event {
            zwp_primary_selection_device_v1::Event::DataOffer { offer } => {
                if observed {
                    client.primary_offers.push((Vec::new(), offer));
                }
            }
            zwp_primary_selection_device_v1::Event::Selection { id, .. } if observed => {
                client.primary_selection = id;
            }
            _ => {}
        }
    }

    /// Same `new_id` story as the data-control devices: `data_offer`
    /// (opcode 0) is the only child-creating event.
    fn event_created_child(
        opcode: u16,
        qh: &QueueHandle<Self>,
    ) -> std::sync::Arc<dyn wayland_client::backend::ObjectData> {
        assert_eq!(
            opcode, 0,
            "the only child-creating event here is data_offer"
        );
        qh.make_data::<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()>(())
    }
}

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()> for Client {
    fn event(
        client: &mut Self,
        _: &zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event
            && let Some((mimes, _)) = client.primary_offers.last_mut()
        {
            mimes.push(mime_type);
        }
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);

/// A memfd the test owns, for an offer's `receive`: the compositor writes the
/// selection into it, the test reads it back from offset zero.
fn receive_memfd() -> std::fs::File {
    use rustix::fs::{MemfdFlags, memfd_create};

    let fd = memfd_create("scoot-selection-test", MemfdFlags::CLOEXEC).expect("a memfd");
    std::fs::File::from(fd)
}

fn read_received(mut file: std::fs::File) -> Vec<u8> {
    file.seek(std::io::SeekFrom::Start(0))
        .expect("a seek to zero");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).expect("the received bytes");
    bytes
}

/// A `size`x`size` `wl_buffer`, over a real memfd -- the same path any
/// toolkit takes to map a window.
fn solid_buffer(shm: &wl_shm::WlShm, qh: &QueueHandle<Client>, size: i32) -> wl_buffer::WlBuffer {
    use rustix::fs::{MemfdFlags, memfd_create};

    let stride = size * 4;
    let len = (stride * size) as usize;
    let fd = memfd_create("scoot-selection-test", MemfdFlags::CLOEXEC).expect("a memfd");
    let mut file = std::fs::File::from(fd);
    file.write_all(&vec![0u8; len]).expect("a filled pool file");
    let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
    let buffer = pool.create_buffer(0, size, size, stride, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    buffer
}

/// Rounds the client's queue until `ready` sees what the test is waiting for,
/// or errors when the compositor never sends it. Each round trip is one full
/// client↔compositor exchange, and the compositor's own thread is pumping
/// concurrently, so a handful is plenty; the bound is against hanging the
/// test suite, not against normal latency.
fn wait_for_event(
    conn: &mut ClientConn,
    what: &str,
    mut ready: impl FnMut(&Client) -> bool,
) -> Result<(), String> {
    for _ in 0..50 {
        conn.roundtrip()?;
        if ready(&conn.client) {
            return Ok(());
        }
    }
    Err(format!("the compositor never sent {what}"))
}

#[test]
fn all_selection_globals_advertised() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|conn| {
        let versions: std::collections::HashMap<&str, u32> = conn
            .client
            .globals
            .iter()
            .map(|(interface, version)| (interface.as_str(), *version))
            .collect();
        for (interface, version) in [
            ("zwlr_data_control_manager_v1", 2),
            ("ext_data_control_manager_v1", 1),
            ("zwp_primary_selection_device_manager_v1", 1),
        ] {
            match versions.get(interface) {
                Some(&advertised) if advertised >= version => {}
                found => {
                    return Err(format!(
                        "{interface} not advertised at version {version} (saw {found:?}); \
                         globals were {:?}",
                        conn.client.globals,
                    ));
                }
            }
        }
        Ok("all three selection globals advertised".into())
    });
    assert_eq!(
        harness.wait_for(handle).as_deref(),
        Ok("all three selection globals advertised"),
    );
}

#[test]
fn primary_selection_round_trip() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let manager = conn
            .client
            .primary_manager
            .clone()
            .ok_or("no zwp_primary_selection_device_manager_v1 -- the global is missing")?;
        let seat = conn.client.seat.clone().ok_or("no wl_seat")?;
        let compositor = conn.client.compositor.clone().ok_or("no wl_compositor")?;
        let shm = conn.client.shm.clone().ok_or("no wl_shm")?;
        let wm_base = conn.client.wm_base.clone().ok_or("no xdg_wm_base")?;
        // Primary selection is focus-gated (unlike data-control): the
        // compositor only accepts `set_selection` from the client holding
        // keyboard focus. So first prove the denial -- a selection set with
        // nobody focused must reach no device -- then map a window and do
        // the real round trip.
        let setter = manager.get_device(&seat, &qh, ());
        let denied_source = manager.create_source(&qh, ());
        denied_source.offer(MIME.to_string());
        setter.set_selection(Some(&denied_source), 0);
        conn.roundtrip()?;
        conn.roundtrip()?;
        if !conn.client.primary_offers.is_empty() {
            return Err("an unfocused set_selection reached a device".into());
        }
        // Map a window: a surface, a toplevel, a configure ack and a real
        // buffer, the way any toolkit maps one.
        let surface = compositor.create_surface(&qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
        let _toplevel = xdg.get_toplevel(&qh, ());
        surface.commit();
        wait_for_event(&mut conn, "an xdg configure", |client| {
            client.xdg_serial.is_some()
        })?;
        xdg.ack_configure(conn.client.xdg_serial.expect("the serial"));
        let buffer = solid_buffer(&shm, &qh, 40);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, 40, 40);
        surface.commit();
        // The new window takes keyboard focus, which is what un-gates the
        // selection below.
        wait_for_event(&mut conn, "keyboard focus", |client| {
            client.keyboard_focus.is_some()
        })?;
        let source = manager.create_source(&qh, ());
        source.offer(MIME.to_string());
        setter.set_selection(Some(&source), 0);
        conn.roundtrip()?;
        // The observer must already exist when the selection is set: unlike
        // the data-control managers, Smithay's primary-selection
        // implementation only broadcasts to devices present at set time and
        // does not backfill a device created afterwards.
        let observer = manager.get_device(&seat, &qh, ());
        conn.client.primary_observer = Some(observer.clone());
        let _ = observer;
        // Re-set now that the observer exists.
        let source = manager.create_source(&qh, ());
        source.offer(MIME.to_string());
        setter.set_selection(Some(&source), 0);
        conn.roundtrip()?;
        wait_for_event(&mut conn, "a primary offer", |client| {
            !client.primary_offers.is_empty()
        })?;
        wait_for_event(&mut conn, "a primary selection", |client| {
            client.primary_selection.is_some()
        })?;
        if conn.client.primary_offers.len() != 1 {
            return Err(format!(
                "expected one offer, saw {}",
                conn.client.primary_offers.len()
            ));
        }
        conn.roundtrip()?;
        let (mimes, _) = &conn.client.primary_offers[0];
        if mimes != &vec![MIME.to_string()] {
            return Err(format!("expected [{MIME}] offered, saw {mimes:?}"));
        }
        let (_, offer) = conn.client.primary_offers.pop().expect("the offer");
        let file = receive_memfd();
        offer.receive(MIME.to_string(), file.as_fd());
        conn.roundtrip()?;
        conn.roundtrip()?;
        let bytes = read_received(file);
        if bytes != PAYLOAD {
            return Err(format!("expected {PAYLOAD:?} back, saw {bytes:?}"));
        }
        Ok("primary selection round trip held its bytes".into())
    });
    assert_eq!(
        harness.wait_for(handle).as_deref(),
        Ok("primary selection round trip held its bytes"),
    );
}

#[test]
fn wlr_data_control_round_trip() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let manager = conn
            .client
            .wlr_manager
            .clone()
            .ok_or("no zwlr_data_control_manager_v1 -- the global is missing")?;
        let seat = conn.client.seat.clone().ok_or("no wl_seat")?;
        // Two devices on one connection: the first sets, the second observes.
        // A fresh device is told the current selection on creation, so the
        // observer is created *after* the set -- the same order a clipboard
        // manager that starts after something was copied sees.
        let setter = manager.get_data_device(&seat, &qh, ());
        let source = manager.create_data_source(&qh, ());
        source.offer(MIME.to_string());
        setter.set_selection(Some(&source));
        conn.roundtrip()?;
        let observer = manager.get_data_device(&seat, &qh, ());
        conn.client.wlr_observer = Some(observer.clone());
        let _ = observer;
        wait_for_event(&mut conn, "a wlr data offer", |client| {
            !client.wlr_offers.is_empty()
        })?;
        wait_for_event(&mut conn, "a wlr selection", |client| {
            client.wlr_selection.is_some()
        })?;
        let offer_count = conn.client.wlr_offers.len();
        if offer_count != 1 {
            return Err(format!("expected one offer, saw {offer_count}"));
        }
        // The mime list arrives as separate `offer` events after the
        // `data_offer` that carried the object; give them a round trip to
        // land, then receive through the selection's own offer.
        conn.roundtrip()?;
        let (mimes, _) = &conn.client.wlr_offers[0];
        if mimes != &vec![MIME.to_string()] {
            return Err(format!("expected [{MIME}] offered, saw {mimes:?}"));
        }
        let (_, offer) = conn.client.wlr_offers.pop().expect("the offer");
        let file = receive_memfd();
        offer.receive(MIME.to_string(), file.as_fd());
        conn.roundtrip()?;
        conn.roundtrip()?;
        let bytes = read_received(file);
        if bytes != PAYLOAD {
            return Err(format!("expected {PAYLOAD:?} back, saw {bytes:?}"));
        }
        Ok("wlr clipboard round trip held its bytes".into())
    });
    assert_eq!(
        harness.wait_for(handle).as_deref(),
        Ok("wlr clipboard round trip held its bytes"),
    );
}

#[test]
fn ext_data_control_round_trip() {
    let mut harness = Harness::new();
    let handle = harness.run_client(|mut conn| {
        let qh = conn.queue.handle();
        let manager = conn
            .client
            .ext_manager
            .clone()
            .ok_or("no ext_data_control_manager_v1 -- the global is missing")?;
        let seat = conn.client.seat.clone().ok_or("no wl_seat")?;
        let setter = manager.get_data_device(&seat, &qh, ());
        let source = manager.create_data_source(&qh, ());
        source.offer(MIME.to_string());
        setter.set_selection(Some(&source));
        conn.roundtrip()?;
        let observer = manager.get_data_device(&seat, &qh, ());
        conn.client.ext_observer = Some(observer.clone());
        let _ = observer;
        wait_for_event(&mut conn, "an ext data offer", |client| {
            !client.ext_offers.is_empty()
        })?;
        wait_for_event(&mut conn, "an ext selection", |client| {
            client.ext_selection.is_some()
        })?;
        conn.roundtrip()?;
        let (mimes, _) = &conn.client.ext_offers[0];
        if mimes != &vec![MIME.to_string()] {
            return Err(format!("expected [{MIME}] offered, saw {mimes:?}"));
        }
        let (_, offer) = conn.client.ext_offers.pop().expect("the offer");
        let file = receive_memfd();
        offer.receive(MIME.to_string(), file.as_fd());
        conn.roundtrip()?;
        conn.roundtrip()?;
        let bytes = read_received(file);
        if bytes != PAYLOAD {
            return Err(format!("expected {PAYLOAD:?} back, saw {bytes:?}"));
        }
        Ok("ext clipboard round trip held its bytes".into())
    });
    assert_eq!(
        harness.wait_for(handle).as_deref(),
        Ok("ext clipboard round trip held its bytes"),
    );
}
