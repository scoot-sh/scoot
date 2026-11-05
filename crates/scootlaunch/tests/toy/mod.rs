//! A mapped `xdg_toplevel` window with a title and an `app_id`, for the
//! windows-mode tests. There is no window client on the test machines
//! (no `foot`), so this runs in a helper thread on its own connection.
//! Dropping a [`Toy`] closes the window (the connection goes away).

#![allow(dead_code)]

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use rustix::event::{PollFd, PollFlags};
use rustix::time::Timespec;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

enum Cmd {
    Stop,
}

/// One mapped window, gone on drop.
pub struct Toy {
    tx: Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

impl Toy {
    /// Maps a window titled `title` with `app_id` on `socket` (the
    /// compositor's Wayland socket path).
    pub fn open(socket: PathBuf, app_id: &str, title: &str) -> Self {
        let (tx, inbox) = mpsc::channel();
        let app_id = app_id.to_owned();
        let title = title.to_owned();
        let thread = thread::spawn(move || run(&socket, &app_id, &title, inbox));
        Self {
            tx,
            thread: Some(thread),
        }
    }

    /// Closes the window and waits for the client to be gone.
    pub fn close(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        // The thread may have gone already (the compositor asked it to
        // close): then there is nothing to tell it.
        let _ = self.tx.send(Cmd::Stop);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

impl Drop for Toy {
    fn drop(&mut self) {
        self.stop();
    }
}

struct State {
    surface: WlSurface,
    buffer: WlBuffer,
    /// Kept open: the buffer reads from it.
    _file: std::fs::File,
    attached: bool,
    gone: bool,
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
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

impl Dispatch<XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
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

impl Dispatch<XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if !state.attached {
                state.surface.attach(Some(&state.buffer), 0, 0);
                state.attached = true;
            }
            state.surface.commit();
        }
    }
}

impl Dispatch<XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            state.gone = true;
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WlBuffer);

fn run(socket: &PathBuf, app_id: &str, title: &str, inbox: Receiver<Cmd>) {
    let stream = UnixStream::connect(socket).expect("toy: no compositor socket");
    let conn = Connection::from_socket(stream).expect("toy: no connection");
    let (globals, mut queue) = registry_queue_init::<State>(&conn).expect("toy: no globals");
    let qh = queue.handle();
    let compositor: WlCompositor = globals.bind(&qh, 4..=6, ()).expect("toy: no compositor");
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).expect("toy: no shm");
    let wm_base: XdgWmBase = globals.bind(&qh, 1..=3, ()).expect("toy: no xdg_wm_base");
    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_app_id(app_id.to_owned());
    toplevel.set_title(title.to_owned());
    let len: u64 = 64 * 64 * 4;
    let fd = rustix::fs::memfd_create(c"toy", rustix::fs::MemfdFlags::CLOEXEC).unwrap();
    rustix::fs::ftruncate(&fd, len).unwrap();
    let mut file = std::fs::File::from(fd);
    std::io::Write::write_all(&mut file, &vec![0x33u8; len as usize]).unwrap();
    let pool = shm.create_pool(file.as_fd(), len as i32, &qh, ());
    let buffer = pool.create_buffer(0, 64, 64, 64 * 4, wl_shm::Format::Xrgb8888, &qh, ());
    surface.commit();
    conn.flush().unwrap();
    let mut state = State {
        surface,
        buffer,
        _file: file,
        attached: false,
        gone: false,
    };
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: 50_000_000,
    };
    loop {
        if let Ok(Cmd::Stop) = inbox.try_recv() {
            return;
        }
        if conn.flush().is_err() {
            break;
        }
        let Some(guard) = queue.prepare_read() else {
            if queue.dispatch_pending(&mut state).is_err() {
                break;
            }
            continue;
        };
        let mut fds = [PollFd::from_borrowed_fd(
            guard.connection_fd(),
            PollFlags::IN,
        )];
        if rustix::event::poll(&mut fds, Some(&timeout)).is_err() {
            drop(guard);
            continue;
        }
        let readable = fds[0]
            .revents()
            .intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR);
        if readable {
            if guard.read().is_err() {
                break;
            }
        } else {
            drop(guard);
        }
        if queue.dispatch_pending(&mut state).is_err() || state.gone {
            break;
        }
    }
}
