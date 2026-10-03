//! A session-lock client (`ext_session_lock_v1`): the smallest one that
//! really locks a compositor, so a test can ask what is on screen, and what
//! a bar may show, while the session is locked. One lock surface per output,
//! each a solid [`COLOR`], on a thread of its own that reads the socket
//! until it is told to stop; dropping it unlocks (a locked session with no
//! client left would stay locked, so it unlocks before it goes).

#![allow(dead_code)]

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use scootbg_mem::ShmBuffer;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_manager_v1::ExtSessionLockManagerV1;
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_surface_v1::{
    self, ExtSessionLockSurfaceV1,
};
use wayland_protocols::ext::session_lock::v1::client::ext_session_lock_v1::{
    self, ExtSessionLockV1,
};

/// What every lock surface is filled with.
pub const COLOR: &str = "#336699";

struct State {
    shm: WlShm,
    compositor: WlCompositor,
    locked: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    surfaces: Vec<(WlSurface, ExtSessionLockSurfaceV1)>,
    buffers: Vec<(ShmBuffer, WlShmPool, WlBuffer)>,
}

pub struct Locker {
    stop: Arc<AtomicBool>,
    locked: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Locker {
    /// Asks the compositor at `socket` to lock the session, and keeps the
    /// lock until dropped.
    pub fn start(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        let conn = Connection::from_socket(stream).unwrap();
        let (globals, mut queue) = registry_queue_init::<State>(&conn).unwrap();
        let qh = queue.handle();
        let locked = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
        let shm: WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
        let manager: ExtSessionLockManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .expect("the compositor has no ext_session_lock_manager_v1");
        let outputs: Vec<WlOutput> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|global| global.interface == "wl_output")
                .map(|global| {
                    globals.registry().bind::<WlOutput, _, _>(
                        global.name,
                        global.version.min(1),
                        &qh,
                        (),
                    )
                })
                .collect()
        });
        let mut state = State {
            shm,
            compositor,
            locked: locked.clone(),
            finished: finished.clone(),
            surfaces: Vec::new(),
            buffers: Vec::new(),
        };
        let lock = manager.lock(&qh, ());
        for output in &outputs {
            let surface = state.compositor.create_surface(&qh, ());
            let role = lock.get_lock_surface(&surface, output, &qh, ());
            state.surfaces.push((surface, role));
        }
        queue.roundtrip(&mut state).unwrap();
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                if queue.roundtrip(&mut state).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            lock.unlock_and_destroy();
            let _ = queue.roundtrip(&mut state);
        });
        Self {
            stop,
            locked,
            finished,
            thread: Some(thread),
        }
    }

    /// The compositor said `locked`: the session is locked now.
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// The compositor refused or ended the lock (`finished`).
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
}

impl Drop for Locker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Dispatch<ExtSessionLockV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => state.locked.store(true, Ordering::Relaxed),
            ext_session_lock_v1::Event::Finished => state.finished.store(true, Ordering::Relaxed),
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        role: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        else {
            return;
        };
        role.ack_configure(serial);
        let Some((surface, _)) = state.surfaces.iter().find(|(_, r)| r == role) else {
            return;
        };
        let mut shm = ShmBuffer::new(width.max(1), height.max(1)).unwrap();
        let [r, g, b] = [0x33u8, 0x66, 0x99];
        for pixel in shm.pixels_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[b, g, r, 0xff]);
        }
        let geometry = shm.geometry();
        let pool = state
            .shm
            .create_pool(shm.fd().unwrap(), geometry.len, qh, ());
        shm.close_fd();
        let buffer = pool.create_buffer(
            0,
            geometry.width,
            geometry.height,
            geometry.stride,
            wl_shm::Format::Xrgb8888,
            qh,
            (),
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, geometry.width, geometry.height);
        surface.commit();
        state.buffers.push((shm, pool, buffer));
    }
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

delegate_noop!(State: WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ignore WlOutput);
delegate_noop!(State: ExtSessionLockManagerV1);
