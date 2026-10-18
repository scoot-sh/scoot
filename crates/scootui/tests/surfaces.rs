//! The on-demand lifecycle against real compositors: a layer surface
//! and a full-screen toplevel created and destroyed in a loop, with fds,
//! RSS and `/proc/self/maps` flat, and the toplevel mapping fullscreen.
//!
//! Needs headless scoot (`SCOOTBAR_REQUIRE_SCOOT=1` turns a skip into a
//! failure) and sway (`SCOOTBAR_REQUIRE_SWAY=1`), as scootbar's own
//! integration tests do. The test process itself is the Wayland client,
//! driving `scootui::surfaces` exactly as scootnotify and scootlaunch
//! will: content arrives, the surface is created with no buffer, the
//! configure is acked, a named buffer is drawn and committed, and when
//! the content clears the surface is destroyed with its buffers
//! unmapped.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use scootui::output::Size;
use scootui::surfaces::{
    Buffers, Demand, Effect, FullscreenSpec, Globals, LayerLive, LayerSpec, Plan, ToplevelLive,
};
use scootui::test_support::Session;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_callback::WlCallback;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor, ZwlrLayerSurfaceV1,
};

/// How long a wait lasts: a headless compositor answers in milliseconds,
/// so this only bounds a wedge.
const PATIENCE: Duration = Duration::from_secs(10);

/// The on-demand client: the bound globals, one demand and its buffers,
/// and what the compositor last said.
struct Client {
    compositor: WlCompositor,
    shm: WlShm,
    layer_shell: Option<ZwlrLayerShellV1>,
    xdg_wm_base: Option<XdgWmBase>,
    viewporter: Option<WpViewporter>,
    fractional: Option<WpFractionalScaleManagerV1>,
    outputs: Vec<(u32, WlOutput)>,
    demand: Demand,
    buffers: Buffers,
    live_layer: Option<LayerLive>,
    live_top: Option<ToplevelLive>,
    layer_configure: Option<(u32, u32, u32)>,
    top_size: Option<(u32, u32)>,
    top_serial: Option<u32>,
    top_acked: bool,
    releases: u32,
}

impl Client {
    fn globals(&self) -> Globals<'_> {
        Globals {
            compositor: &self.compositor,
            layer_shell: self.layer_shell.as_ref(),
            xdg_wm_base: self.xdg_wm_base.as_ref(),
            viewporter: self.viewporter.as_ref(),
            fractional_scale: self.fractional.as_ref(),
        }
    }

    /// Carries out a layer effect the way an app's glue does.
    fn carry_layer(&mut self, effect: Effect) {
        match effect {
            Effect::None | Effect::Ack(_) => {}
            Effect::Create => {}
            Effect::Destroy | Effect::DestroyAndRetry | Effect::DestroyAndGiveUp => {
                self.buffers.clear();
                if let Some(live) = self.live_layer.take() {
                    live.destroy();
                }
            }
        }
    }

    /// Acks a pending toplevel configure with the last size the compositor
    /// sent (or 0,0 while none arrived: the compositor sends the
    /// toplevel's size before the surface's serial, but a first
    /// `xdg_surface.configure` may land before any size, and the
    /// fullscreen size itself may arrive one configure later, as under
    /// sway). Every serial is acked exactly once, at once.
    fn ack_top(&mut self) {
        let Some(serial) = self.top_serial else {
            return;
        };
        self.top_serial = None;
        let (w, h) = self.top_size.unwrap_or((0, 0));
        let effect = self.demand.configure(serial, w, h);
        assert!(matches!(effect, Effect::Ack(_)));
        if let Some(live) = &self.live_top {
            live.ack(serial);
        }
        self.top_acked = true;
    }
}

fn optional<I>(bound: Result<I, wayland_client::globals::BindError>) -> Option<I> {
    bound.ok()
}

/// Binds what the tests need; `None` where the compositor lacks a global.
fn connect(session: &Session) -> (Connection, EventQueue<Client>, QueueHandle<Client>, Client) {
    let stream = UnixStream::connect(session.wayland_socket()).unwrap();
    let conn = Connection::from_socket(stream).unwrap();
    let (list, queue) = registry_queue_init::<Client>(&conn).unwrap();
    let qh = queue.handle();
    let compositor: WlCompositor = list.bind(&qh, 1..=6, ()).unwrap();
    let shm: WlShm = list.bind(&qh, 1..=1, ()).unwrap();
    let layer_shell = optional(list.bind(&qh, 1..=4, ()));
    let xdg_wm_base = optional(list.bind(&qh, 1..=6, ()));
    let viewporter = optional(list.bind(&qh, 1..=1, ()));
    let fractional = if viewporter.is_some() {
        optional(list.bind(&qh, 1..=1, ()))
    } else {
        None
    };
    let mut outputs = Vec::new();
    list.contents().with_list(|advertised| {
        for global in advertised {
            if global.interface == WlOutput::interface().name {
                let output: WlOutput = list.registry().bind::<WlOutput, _, _>(
                    global.name,
                    global.version.min(4),
                    &qh,
                    (),
                );
                outputs.push((global.name, output));
            }
        }
    });
    let client = Client {
        compositor,
        shm,
        layer_shell,
        xdg_wm_base,
        viewporter,
        fractional,
        outputs,
        demand: Demand::new(),
        buffers: Buffers::new(c"scootui-test"),
        live_layer: None,
        live_top: None,
        layer_configure: None,
        top_size: None,
        top_serial: None,
        top_acked: false,
        releases: 0,
    };
    (conn, queue, qh, client)
}

/// One round trip: flushes what is queued and dispatches until the sync
/// lands, so every answer to it (configures, releases) is in.
fn roundtrip(queue: &mut EventQueue<Client>, client: &mut Client) {
    queue.roundtrip(client).unwrap();
}

/// Dispatches until `reached` holds or `PATIENCE` runs out. Polls
/// rather than blocking: a compositor that answers nothing more must
/// fail the wait loudly, not wedge the test.
fn wait_for(
    queue: &mut EventQueue<Client>,
    client: &mut Client,
    what: &str,
    mut reached: impl FnMut(&Client) -> bool,
) {
    let deadline = Instant::now() + PATIENCE;
    while !reached(client) {
        queue.dispatch_pending(client).unwrap();
        if reached(client) {
            break;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn size(width: u32, height: u32) -> Size {
    Size { width, height }
}

/// This process's open fds and RSS: what the acceptance samples flat.
fn self_fds() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}

fn self_rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .unwrap()
}

fn maps_named(name: &str) -> usize {
    std::fs::read_to_string("/proc/self/maps")
        .unwrap()
        .lines()
        .filter(|line| line.contains(name))
        .count()
}

/// The app binary under test: a path in the build directory, so
/// `Session` finds the `scoot` beside it. This test binary itself runs
/// from `debug/deps`, one level too deep, so the lookup goes one up.
fn app() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let mut dir = exe.parent().unwrap().to_owned();
    if dir.file_name().is_some_and(|name| name == "deps") {
        dir = dir.parent().unwrap().to_owned();
    }
    dir.join("scootui-surfaces-test")
}

/// The three tests never overlap: each samples this process's own fds,
/// RSS and maps, which a sibling test's compositor children and memfds
/// would pollute under `cargo test`'s shared-process runner (nextest
/// isolates each test in its own process and needs no help).
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One on-demand layer cycle: content arrives, the surface maps, a named
/// buffer shows it, the content clears and everything is destroyed.
#[allow(clippy::too_many_arguments)]
fn cycle_layer(
    queue: &mut EventQueue<Client>,
    qh: &QueueHandle<Client>,
    client: &mut Client,
    spec: &LayerSpec<'_>,
    content: (u32, u32),
    check_maps: bool,
) {
    client.layer_configure = None;
    assert_eq!(client.demand.show(), Effect::Create);
    let live = LayerLive::create(&client.globals(), None, spec, qh, ()).unwrap();
    client.live_layer = Some(live);
    // The commit is flushed here; the configure arrives, and the
    // dispatch handler acks it at once. This body only reads what the
    // handler recorded: acking twice is a protocol error.
    roundtrip(queue, client);
    let (_serial, w, h) = client.layer_configure.expect("no layer configure");
    let size = size(
        if w == 0 { content.0 } else { w },
        if h == 0 { content.1 } else { h },
    );
    let Plan::Draw(frame) = client.demand.plan(Some(size), true) else {
        panic!("no draw planned for the configured surface");
    };
    let dims = client
        .demand
        .buffer_dims(frame.size)
        .expect("no buffer dims");
    let slot = client
        .buffers
        .take(&client.shm, qh, dims, wl_shm::Format::Xrgb8888, ())
        .unwrap()
        .expect("no free buffer while the compositor should hold none");
    slot.pixels_mut().fill(0x11);
    if check_maps {
        assert!(
            maps_named("/memfd:scootui-test") >= 1,
            "the drawn buffer is not named for the app"
        );
    }
    client
        .live_layer
        .as_ref()
        .unwrap()
        .surface()
        .attach(Some(slot.buffer()), 0, 0);
    client
        .live_layer
        .as_mut()
        .unwrap()
        .prepare(frame.size, frame.scale);
    client.live_layer.as_ref().unwrap().surface().commit();
    slot.mark_committed();
    client.demand.drew(frame);
    // A second frame on the same surface: the superseded buffer's
    // release arrives, freeing its slot for the next cycle.
    let Plan::Draw(frame) = client.demand.plan(Some(size), true) else {
        panic!("no second draw planned for the configured surface");
    };
    let dims = client
        .demand
        .buffer_dims(frame.size)
        .expect("no buffer dims");
    let slot = client
        .buffers
        .take(&client.shm, qh, dims, wl_shm::Format::Xrgb8888, ())
        .unwrap()
        .expect("no free buffer while one should have been released");
    slot.pixels_mut().fill(0x33);
    client
        .live_layer
        .as_ref()
        .unwrap()
        .surface()
        .attach(Some(slot.buffer()), 0, 0);
    client
        .live_layer
        .as_mut()
        .unwrap()
        .prepare(frame.size, frame.scale);
    client.live_layer.as_ref().unwrap().surface().commit();
    slot.mark_committed();
    client.demand.drew(frame);
    // Lets the compositor map the surface (and release past buffers).
    roundtrip(queue, client);
    assert_eq!(client.demand.hide(), Effect::Destroy);
    client.carry_layer(Effect::Destroy);
    // The destroy is flushed here; a release racing it is ignored.
    roundtrip(queue, client);
}

#[test]
fn layer_surface_cycles_leave_no_trace_under_scoot() {
    let _serial = SERIAL.lock().unwrap();
    let Some(session) = Session::scoot(&app(), "layer-cycle", 1, "") else {
        return;
    };
    let (_conn, mut queue, qh, mut client) = connect(&session);
    assert!(
        client.layer_shell.is_some(),
        "headless scoot has no layer shell"
    );
    let spec = LayerSpec {
        namespace: "scootui-test",
        exclusive_keyboard: false,
        anchor: Anchor::Top | Anchor::Right,
        width: 400,
        height: 200,
        margin: [8, 8, 8, 8],
        exclusive_zone: 0,
    };
    // Warms the allocator and the connection before sampling.
    for _ in 0..5 {
        cycle_layer(&mut queue, &qh, &mut client, &spec, (400, 200), false);
    }
    assert_eq!(maps_named("/memfd:scootui-test"), 0);
    let (fds_before, rss_before) = (self_fds(), self_rss_kb());
    for round in 0..1000u32 {
        // The stack grows halfway through: the surface follows it.
        if round == 500 {
            client.layer_configure = None;
            assert_eq!(client.demand.show(), Effect::Create);
            let live = LayerLive::create(&client.globals(), None, &spec, &qh, ()).unwrap();
            client.live_layer = Some(live);
            roundtrip(&mut queue, &mut client);
            client.layer_configure.expect("no layer configure");
            client.live_layer.as_ref().unwrap().set_size(400, 100);
            client.live_layer.as_ref().unwrap().surface().commit();
            roundtrip(&mut queue, &mut client);
            let (_serial, w, h) = client.layer_configure.expect("no resized configure");
            assert_eq!((w, h), (400, 100), "the resize did not land");
            assert_eq!(client.demand.hide(), Effect::Destroy);
            client.carry_layer(Effect::Destroy);
            roundtrip(&mut queue, &mut client);
            continue;
        }
        cycle_layer(&mut queue, &qh, &mut client, &spec, (400, 200), round == 10);
    }
    assert_eq!(maps_named("/memfd:scootui-test"), 0);
    let (fds_after, rss_after) = (self_fds(), self_rss_kb());
    assert_eq!(fds_after, fds_before, "fds grew over 1,000 cycles");
    assert!(
        rss_after <= rss_before + 512,
        "RSS grew over 1,000 cycles: {rss_before} -> {rss_after} KiB"
    );
    assert!(client.releases > 0, "no buffer release ever arrived");
}

#[test]
fn toplevel_maps_fullscreen_under_scoot() {
    let _serial = SERIAL.lock().unwrap();
    let Some(session) = Session::scoot(&app(), "top-full", 1, "") else {
        return;
    };
    let (_conn, mut queue, qh, mut client) = connect(&session);
    assert!(
        client.xdg_wm_base.is_some(),
        "headless scoot has no xdg_wm_base"
    );
    let spec = FullscreenSpec {
        app_id: "scootui-test",
        title: "test",
    };
    let fds_before = self_fds();
    assert_eq!(client.demand.show(), Effect::Create);
    let live = ToplevelLive::create(&client.globals(), None, &spec, &qh, ()).unwrap();
    client.live_top = Some(live);
    roundtrip(&mut queue, &mut client);
    // Fullscreen on the headless output: the compositor sizes it.
    assert_eq!(
        client.top_size,
        Some((1600, 1000)),
        "the toplevel did not map fullscreen"
    );
    assert!(client.top_acked, "the configure was never acked");
    // Draws once, like a greeter paints its screen.
    let frame = match client.demand.plan(Some(size(1600, 1000)), true) {
        Plan::Draw(frame) => frame,
        Plan::Commit | Plan::Nothing => panic!("no draw planned for the mapped toplevel"),
    };
    let dims = client
        .demand
        .buffer_dims(frame.size)
        .expect("no buffer dims");
    let slot = client
        .buffers
        .take(&client.shm, &qh, dims, wl_shm::Format::Xrgb8888, ())
        .unwrap()
        .expect("no free buffer");
    slot.pixels_mut().fill(0x22);
    assert!(
        maps_named("/memfd:scootui-test") >= 1,
        "the drawn buffer is not named for the app"
    );
    client
        .live_top
        .as_ref()
        .unwrap()
        .surface()
        .attach(Some(slot.buffer()), 0, 0);
    client
        .live_top
        .as_mut()
        .unwrap()
        .prepare(frame.size, frame.scale);
    client.live_top.as_ref().unwrap().surface().commit();
    slot.mark_committed();
    client.demand.drew(frame);
    roundtrip(&mut queue, &mut client);
    // Destroyed with its buffers unmapped: nothing held at idle.
    assert_eq!(client.demand.hide(), Effect::Destroy);
    client.buffers.clear();
    client.live_top.take().unwrap().destroy();
    roundtrip(&mut queue, &mut client);
    assert_eq!(maps_named("/memfd:scootui-test"), 0);
    assert_eq!(self_fds(), fds_before, "a toplevel leaked an fd");
}

/// Finds the sway tree node with this app id, if mapped.
fn find_app_id<'a>(node: &'a serde_json::Value, app_id: &str) -> Option<&'a serde_json::Value> {
    if node.get("app_id").and_then(|id| id.as_str()) == Some(app_id) {
        return Some(node);
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = node.get(key).and_then(|kids| kids.as_array()) {
            for child in children {
                if let Some(found) = find_app_id(child, app_id) {
                    return Some(found);
                }
            }
        }
    }
    None
}

#[test]
fn surfaces_map_under_sway() {
    let _serial = SERIAL.lock().unwrap();
    let Some(session) = Session::sway(&app(), "surfaces", 1) else {
        return;
    };
    let (_conn, mut queue, qh, mut client) = connect(&session);
    assert!(client.layer_shell.is_some(), "sway has no layer shell");
    assert!(client.xdg_wm_base.is_some(), "sway has no xdg_wm_base");
    // The panel variant: exclusive keyboard, like scootlaunch's.
    let panel = LayerSpec {
        namespace: "scootui-test",
        exclusive_keyboard: true,
        anchor: Anchor::Top,
        width: 600,
        height: 300,
        margin: [0, 0, 0, 0],
        exclusive_zone: 0,
    };
    for _ in 0..5 {
        cycle_layer(&mut queue, &qh, &mut client, &panel, (600, 300), false);
    }
    // The greeter variant: a full-screen toplevel.
    let spec = FullscreenSpec {
        app_id: "scootui-test",
        title: "test",
    };
    assert_eq!(client.demand.show(), Effect::Create);
    let output = client.outputs.first().map(|(_, output)| output.clone());
    let live = ToplevelLive::create(&client.globals(), output.as_ref(), &spec, &qh, ()).unwrap();
    client.live_top = Some(live);
    roundtrip(&mut queue, &mut client);
    assert!(client.top_acked, "the toplevel never configured under sway");
    // Sway sizes a fullscreen toplevel once it maps, not at map time, so
    // map with a provisional frame first and wait for the real size.
    let provisional = size(256, 256);
    let dims = client
        .demand
        .buffer_dims(provisional)
        .expect("no provisional dims");
    let slot = client
        .buffers
        .take(&client.shm, &qh, dims, wl_shm::Format::Xrgb8888, ())
        .unwrap()
        .expect("no provisional buffer");
    slot.pixels_mut().fill(0x44);
    client
        .live_top
        .as_ref()
        .unwrap()
        .surface()
        .attach(Some(slot.buffer()), 0, 0);
    client.live_top.as_ref().unwrap().surface().commit();
    slot.mark_committed();
    roundtrip(&mut queue, &mut client);
    wait_for(&mut queue, &mut client, "the fullscreen size", |client| {
        client.top_size.is_some_and(|(w, h)| w > 0 && h > 0)
    });
    let tree: serde_json::Value =
        serde_json::from_str(&session.swaymsg(&["-t", "get_tree"])).unwrap();
    let node = find_app_id(&tree, "scootui-test").expect("the toplevel is not in sway's tree");
    assert_eq!(
        node.get("fullscreen_mode"),
        Some(&serde_json::Value::from(1)),
        "the toplevel is not fullscreen under sway"
    );
    assert_eq!(client.demand.hide(), Effect::Destroy);
    client.buffers.clear();
    client.live_top.take().unwrap().destroy();
    roundtrip(&mut queue, &mut client);
    assert_eq!(maps_named("/memfd:scootui-test"), 0);
    // Showing again after the destroy maps cleanly: no surface held.
    cycle_layer(&mut queue, &qh, &mut client, &panel, (600, 300), false);
}

impl Dispatch<WlRegistry, GlobalListContents> for Client {
    fn event(
        client: &mut Self,
        registry: &WlRegistry,
        event: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wayland_client::protocol::wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                if interface == WlOutput::interface().name
                    && !client.outputs.iter().any(|(known, _)| *known == name)
                {
                    let output: WlOutput =
                        registry.bind::<WlOutput, _, _>(name, version.min(4), qh, ());
                    client.outputs.push((name, output));
                }
            }
            wayland_client::protocol::wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = client.outputs.iter().position(|(known, _)| *known == name) {
                    let (_, output) = client.outputs.remove(index);
                    output.release();
                    // The output shown on is gone: destroy, no retry.
                    let effect = client.demand.output_removed();
                    client.carry_layer(effect);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, ()> for Client {
    fn event(
        _: &mut Self,
        _: &WlOutput,
        _: wayland_client::protocol::wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, ()> for Client {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: wayland_client::protocol::wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShm, ()> for Client {
    fn event(
        _: &mut Self,
        _: &WlShm,
        _: wayland_client::protocol::wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShmPool, ()> for Client {
    fn event(
        _: &mut Self,
        _: &WlShmPool,
        _: wayland_client::protocol::wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for Client {
    fn event(
        client: &mut Self,
        surface: &WlSurface,
        event: wayland_client::protocol::wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Only the live surface's events count: one destroyed after a
        // `closed` may still have events in flight.
        let live = client
            .live_layer
            .as_ref()
            .map(LayerLive::surface)
            .or_else(|| client.live_top.as_ref().map(ToplevelLive::surface));
        if live != Some(surface) {
            return;
        }
        if let wayland_client::protocol::wl_surface::Event::PreferredBufferScale { factor } = event
        {
            client.demand.prefer_buffer_scale(factor);
        }
    }
}

impl Dispatch<WpViewport, ()> for Client {
    fn event(
        _: &mut Self,
        _: &WpViewport,
        _: wayland_protocols::wp::viewporter::client::wp_viewport::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpFractionalScaleV1, ()> for Client {
    fn event(
        client: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            client.demand.prefer_fractional(scale);
        }
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Client {
    fn event(
        client: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if client.live_layer.as_ref().map(LayerLive::role) != Some(layer) {
            return;
        }
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                client.layer_configure = Some((serial, width, height));
                let effect = client.demand.configure(serial, width, height);
                assert!(matches!(effect, Effect::Ack(_)));
                if let Some(live) = &client.live_layer {
                    live.ack(serial);
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                let effect = client.demand.closed();
                client.carry_layer(effect);
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgWmBase, ()> for Client {
    fn event(
        _: &mut Self,
        base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Missing this kills the client.
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for Client {
    fn event(
        client: &mut Self,
        xdg: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if client.live_top.as_ref().map(ToplevelLive::xdg_surface) != Some(xdg) {
            return;
        }
        if let xdg_surface::Event::Configure { serial } = event {
            client.top_serial = Some(serial);
            client.ack_top();
        }
    }
}

impl Dispatch<XdgToplevel, ()> for Client {
    fn event(
        client: &mut Self,
        toplevel: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if client.live_top.as_ref().map(ToplevelLive::toplevel) != Some(toplevel) {
            return;
        }
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                // A negative side is a broken compositor: 0, resolved
                // against the content like a layer configure's.
                let sane = |side: i32| u32::try_from(side).unwrap_or(0);
                client.top_size = Some((sane(width), sane(height)));
                client.ack_top();
            }
            xdg_toplevel::Event::Close => {
                let effect = client.demand.closed();
                client.buffers.clear();
                if let Some(live) = client.live_top.take() {
                    live.destroy();
                }
                let _ = effect;
            }
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, ()> for Client {
    fn event(
        client: &mut Self,
        buffer: &WlBuffer,
        event: wayland_client::protocol::wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wayland_client::protocol::wl_buffer::Event::Release = event else {
            return;
        };
        // A release for a buffer destroyed with the surface is ignored:
        // the compositor may release after the surface is gone.
        client.releases += 1;
        client.buffers.released(buffer);
    }
}

impl Dispatch<WlCallback, ()> for Client {
    fn event(
        _: &mut Self,
        _: &WlCallback,
        _: wayland_client::protocol::wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Client: ignore WpViewporter);
delegate_noop!(Client: ignore WpFractionalScaleManagerV1);
