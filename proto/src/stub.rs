//! The already-running single-instance app: a plain `xdg_toplevel` that
//! prints when the compositor moves keyboard focus to it. The panel (or the
//! harness) hands it an activation token over a unix socket, or it redeems
//! `$XDG_ACTIVATION_TOKEN` on its first configure.

use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixListener;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::protocol::{
    wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_registry::WlRegistry,
    wl_shm::{Format as ShmFormat, WlShm}, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols::xdg::shell::client::{
    xdg_surface::{Event as XdgSurfaceEvent, XdgSurface},
    xdg_toplevel::{Event as ToplevelEvent, XdgToplevel},
    xdg_wm_base::{Event as WmBaseEvent, XdgWmBase},
};

use crate::shm::Shm;

pub struct StubOpts {
    pub app_id: String,
    pub sock: Option<std::path::PathBuf>,
    pub quit_after_ms: Option<u64>,
}

struct Stub {
    t0: Instant,
    log: Option<std::fs::File>,
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    wm_base: Option<XdgWmBase>,
    activation: Option<XdgActivationV1>,
    surface: Option<WlSurface>,
    xdg_surface: Option<XdgSurface>,
    toplevel: Option<XdgToplevel>,
    buffer: Option<(WlShmPool, WlBuffer)>,
    configured: bool,
    activated: bool,
    presented: bool,
    env_token: Option<String>,
    redeemed: bool,
    should_exit: bool,
}

impl Stub {
    fn retrace(&mut self, msg: &str) {
        let ms = self.t0.elapsed().as_secs_f64() * 1000.0;
        if let Some(f) = &mut self.log {
            let _ = writeln!(f, "{ms:10.3} {msg}");
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Stub>) {
        const W: u32 = 240;
        const H: u32 = 160;
        let (Some(shm_global), Some(surface)) = (self.shm.clone(), self.surface.clone()) else {
            return;
        };
        if self.buffer.is_some() {
            return;
        }
        let Some(mut mem) = Shm::new(W, H) else { return };
        // Solid magenta: unmistakable in screenshots.
        for px in mem.pixels().chunks_exact_mut(4) {
            px[0] = 0x33;
            px[1] = 0x00;
            px[2] = 0xcc;
            px[3] = 0xff;
        }
        let pool = shm_global.create_pool(mem.memfd.as_fd(), (W * H * 4) as i32, qh, ());
        let buffer =
            pool.create_buffer(0, W as i32, H as i32, (W * 4) as i32, ShmFormat::Argb8888, qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, W as i32, H as i32);
        surface.commit();
        self.buffer = Some((pool, buffer));
    }
}

impl Dispatch<WlRegistry, ()> for Stub {
    fn event(
        st: &mut Stub,
        registry: &WlRegistry,
        event: <WlRegistry as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Stub>,
    ) {
        use wayland_client::protocol::wl_registry::Event as RegEvent;
        if let RegEvent::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    st.compositor = Some(registry.bind::<WlCompositor, _, _>(name, version.min(6), qh, ()));
                }
                "wl_shm" => {
                    st.shm = Some(registry.bind::<WlShm, _, _>(name, 1, qh, ()));
                }
                "xdg_wm_base" => {
                    st.wm_base = Some(registry.bind::<XdgWmBase, _, _>(name, version.min(6), qh, ()));
                }
                "xdg_activation_v1" => {
                    st.activation = Some(registry.bind::<XdgActivationV1, _, _>(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(Stub: ignore WlCompositor);
delegate_noop!(Stub: ignore WlShm);
delegate_noop!(Stub: ignore WlShmPool);
delegate_noop!(Stub: ignore WlSurface);
delegate_noop!(Stub: ignore WlBuffer);
delegate_noop!(Stub: ignore XdgActivationV1);

impl Dispatch<XdgWmBase, ()> for Stub {
    fn event(
        _: &mut Stub,
        wm: &XdgWmBase,
        event: WmBaseEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Stub>,
    ) {
        if let WmBaseEvent::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for Stub {
    fn event(
        st: &mut Stub,
        xdg: &XdgSurface,
        event: XdgSurfaceEvent,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Stub>,
    ) {
        if let XdgSurfaceEvent::Configure { serial } = event {
            xdg.ack_configure(serial);
            st.configured = true;
            st.retrace("STUB_CONFIGURE");
            if st.buffer.is_none() {
                st.draw(qh);
                st.retrace("STUB_FIRST_DRAW");
            }
            if !st.redeemed {
                if let Some(tok) = st.env_token.clone() {
                    // Fresh-spawn path: redeem the token the launcher set.
                    if let (Some(act), Some(surface)) =
                        (st.activation.clone(), st.surface.clone())
                    {
                        act.activate(tok, &surface);
                        st.retrace("STUB_REDEEM_ENV");
                    }
                    st.redeemed = true;
                }
            }
            if let Some((_, buf)) = st.buffer.as_ref() {
                if let Some(surface) = st.surface.clone() {
                    surface.attach(Some(buf), 0, 0);
                    surface.commit();
                }
            }
        }
    }
}

impl Dispatch<XdgToplevel, ()> for Stub {
    fn event(
        st: &mut Stub,
        _: &XdgToplevel,
        event: ToplevelEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Stub>,
    ) {
        match event {
            ToplevelEvent::Configure { states, .. } => {
                // Wayland 0.31 decodes xdg_toplevel states as raw bytes.
                let raw = format!("{states:?}");
                if raw.contains("Activated") && !st.activated {
                    st.activated = true;
                    st.retrace("STUB_ACTIVATED");
                    println!("ACTIVATED");
                }
            }
            ToplevelEvent::Close => {
                st.should_exit = true;
            }
            _ => {}
        }
    }
}

pub fn run(opts: StubOpts, trace: Option<std::path::PathBuf>) -> i32 {
    let t0 = Instant::now();
    let mut log = trace.and_then(|p| {
        std::fs::OpenOptions::new().create(true).append(true).open(p).ok()
    });
fn retrace(log: &mut Option<std::fs::File>, t0: Instant, msg: &str) {
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    if let Some(f) = log {
        let _ = writeln!(f, "{ms:10.3} {msg}");
    }
}
    let conn = Connection::connect_to_env().expect("wayland connect");
    let mut queue: EventQueue<Stub> = conn.new_event_queue();
    let qh = queue.handle();
    let mut st = Stub {
        t0,
        log,
        compositor: None,
        shm: None,
        wm_base: None,
        activation: None,
        surface: None,
        xdg_surface: None,
        toplevel: None,
        buffer: None,
        configured: false,
        activated: false,
        presented: false,
        env_token: std::env::var("XDG_ACTIVATION_TOKEN").ok().filter(|s| !s.is_empty()),
        redeemed: false,
        should_exit: false,
    };
    let _registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut st).expect("roundtrip");
    let compositor = st.compositor.clone().expect("no wl_compositor");
    let wm_base = st.wm_base.clone().expect("no xdg_wm_base");
    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_app_id(opts.app_id.clone());
    toplevel.set_title("w9 stub".into());
    surface.commit();
    st.surface = Some(surface);
    st.xdg_surface = Some(xdg_surface);
    st.toplevel = Some(toplevel);
    // The first draw waits for the initial configure: attaching a buffer
    // before acking it is a protocol error.
    st.retrace("STUB_START");
    println!("READY");

    let listener = opts.sock.as_ref().and_then(|path| {
        let _ = std::fs::remove_file(path);
        UnixListener::bind(path).ok()
    });
    if let Some(l) = &listener {
        l.set_nonblocking(true).ok();
    }
    let backend = conn.backend();
    let wayland_fd = backend.poll_fd();
    let quit_at = opts.quit_after_ms.map(|ms| t0 + Duration::from_millis(ms));
    loop {
        queue.dispatch_pending(&mut st).expect("dispatch");
        if st.should_exit {
            break;
        }
        if let Some(end) = quit_at {
            if Instant::now() >= end {
                st.retrace("STUB_QUIT_TIMER");
                break;
            }
        }
        // Socket orders: the panel's activation token arrives here.
        if let Some(l) = &listener {
            while let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 512];
                if let Ok(n) = s.read(&mut buf) {
                    let line = String::from_utf8_lossy(&buf[..n]);
                    if let Some(tok) = line.strip_prefix("ACTIVATE ") {
                        let tok = tok.trim().to_string();
                        if let (Some(act), Some(surface)) =
                            (st.activation.clone(), st.surface.clone())
                        {
                            act.activate(tok.clone(), &surface);
                            st.retrace(&format!("STUB_ACTIVATE len={}", tok.len()));
                        }
                    }
                }
            }
        }
        if !st.presented && st.configured {
            st.presented = true;
            st.retrace("STUB_PRESENTED");
            println!("PRESENTED");
        }
        conn.flush().expect("flush");
        let timeout = Timespec { tv_sec: 0, tv_nsec: 100_000_000 };
        let mut fds = [PollFd::from_borrowed_fd(wayland_fd.as_fd(), PollFlags::IN)];
        let _ = poll(&mut fds, Some(&timeout));
        if let Some(guard) = queue.prepare_read() {
            let _ = guard.read();
        }
    }
    0
}
