//! Wire tests for dma-buf plane fds: every `add` is one fd, counted for as
//! long as it is open, including after the `wl_buffer` it became is destroyed
//! while a surface still has it committed.
//!
//! Three shapes. Planes on tagged memfds, whose fds in this process's table
//! can be counted by name, for the per-`add` count (the renderer is
//! irrelevant: the fds are counted on arrival). A real one-page udmabuf on
//! pixman, which imports it, for a committed buffer outliving its object.
//! And a real three-plane `YU12` buffer on GLES, the renderer that imports
//! multi-plane buffers, from a dumb buffer on `/dev/dri/card0` (the one
//! provenance the dev VM's software GLES driver takes; see
//! `dmabuf/tests/layouts.rs`), run to the fd bound.

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use smithay::backend::allocator::{Format, Fourcc, Modifier};
use smithay::reexports::drm;
use smithay::reexports::drm::buffer::Buffer as _;
use smithay::reexports::drm::control::Device as _;
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
};

use super::super::{Kind, MAX_FDS_PER_CLIENT};
use crate::cli::RendererKind;
use crate::compositor::decorations::Appearance;
use crate::compositor::dispatch::tests::{ensure_dispatch_flood_headroom, hold_flood_lock};
use crate::compositor::test_support::Harness;

/// `wl_display.error.no_memory`: what a refused `add` disconnects with.
const NO_MEMORY: u32 = 2;

/// The side of the `YU12` test buffer. Even, so its chroma planes have whole
/// samples.
const SIDE: u32 = 32;

enum Step {
    /// `params` params objects of `planes` tagged memfd planes each, then
    /// each consumed by the asynchronous `create`. The memfds are not
    /// dma-bufs, so the renderer refuses every import (`failed`) and the
    /// fds close; before that, each `add` was counted.
    AddAndCreate { params: u32, planes: u32 },
    /// Like [`Step::AddAndCreate`], but stop before `create`: the planes stay
    /// pending in live params objects.
    AddOnly { params: u32, planes: u32 },
    /// Consume every params object [`Step::AddOnly`] left with `create`.
    CreateAll,
    /// A one-page udmabuf through `create_immed`, attached and committed on
    /// a fresh surface, then the buffer and params destroyed. Answers
    /// [`Ack::NoDevice`] where this machine cannot make one.
    CommitUdmabuf,
    /// `count` three-plane `YU12` dumb buffers, each through `create_immed`
    /// on a fresh surface, committed, then its buffer and params destroyed.
    /// A round trip after each, so a kill says how many were held. With
    /// `dups`, this side keeps that many extra fds on each buffer's file
    /// open across its import: fds that name the buffer in this process
    /// (the harness's client is in-process) but that no renderer made, the
    /// way a client could park fds in scoot's received-fd queue. With
    /// `pool`, it first opens a `wl_shm` pool on each buffer's fd and keeps
    /// it: a pool whose fd can close on Smithay's drop thread mid-import.
    CommitYuv { count: u32, dups: u32, pool: bool },
    /// `params` params objects, each with the three `YU12` planes of a fresh
    /// dumb buffer added and kept, not yet created. A round trip after each,
    /// so a kill says how many were complete.
    HoldYuvParams { params: u32 },
    /// `create_immed` every params object [`Step::HoldYuvParams`] kept, each
    /// attached and committed on a fresh surface, the buffer then destroyed.
    ImmedHeldParams,
    /// Destroy every surface the steps above kept.
    DestroySurfaces,
}

enum Ack {
    Done,
    /// This machine cannot make the buffer the step asked for.
    NoDevice(String),
}

type Fixture = Harness<Step, Ack>;

fn start(renderer: RendererKind, tag: &'static str) -> Fixture {
    let mut fixture = Harness::headless_on(Appearance::default(), 32, renderer);
    fixture.spawn(move |stream, steps, acks| run_client(stream, steps, acks, tag));
    fixture
}

impl Fixture {
    fn done(&mut self, step: Step) -> bool {
        match self.run(step) {
            Ack::Done => true,
            Ack::NoDevice(reason) => {
                eprintln!("skipped: {reason}");
                false
            }
        }
    }

    /// Sweeps until the ledger finds `expected` fds of `kind` still open. A
    /// dropped dma-buf closes its planes as the last reference goes, which
    /// may be a dispatch or two after the object that held it.
    fn settle_until_counted(&mut self, kind: Kind, expected: u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let counted = self.state.client_fds.in_flight(Some(kind));
            if counted == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{counted} {kind:?} fds still counted, expected {expected}"
            );
            self.settle();
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// How many fds in this process's table are memfds named with `tag`.
fn held_fds(tag: &str) -> usize {
    let needle = format!("/memfd:{tag} ");
    std::fs::read_dir("/proc/self/fd")
        .expect("/proc/self/fd")
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|link| link.to_string_lossy().starts_with(&needle))
        .count()
}

/// A dma-buf of four planes is four fds kept here, not one: each `add` is
/// counted on arrival, pending in the params object and then in the
/// `Dmabuf`, until the fd closes.
#[test]
fn every_plane_is_its_own_counted_fd() {
    const TAG: &str = "scoot-cfd-planes";
    let mut fixture = start(RendererKind::Pixman, TAG);
    fixture.done(Step::AddOnly {
        params: 3,
        planes: 4,
    });
    let client = fixture.client(0).id();
    assert_eq!(held_fds(TAG), 12);
    assert_eq!(fixture.state.client_fds.held_by(&client), 12);
    assert_eq!(fixture.state.client_fds.in_flight(Some(Kind::Plane)), 12);
    fixture.done(Step::CreateAll);
    fixture.settle_until_counted(Kind::Plane, 0);
    assert_eq!(held_fds(TAG), 0, "the refused imports' planes closed");
}

/// Consuming params with `create` keeps nothing counted once the refused
/// import drops its planes, however many times a client does it: a client
/// the renderer keeps refusing is not ratcheted toward its bound.
#[test]
fn refused_imports_do_not_ratchet_the_count() {
    const TAG: &str = "scoot-cfd-refused";
    let mut fixture = start(RendererKind::Pixman, TAG);
    for _ in 0..8 {
        fixture.done(Step::AddAndCreate {
            params: 32,
            planes: 4,
        });
    }
    fixture.settle_until_counted(Kind::Plane, 0);
    assert_eq!(held_fds(TAG), 0);
}

/// A committed dma-buf outlives its `wl_buffer` the way a pool does: the
/// surface's renderer state holds the buffer, whose `Dmabuf` holds the plane.
/// Counted while the surface keeps it, and not after.
#[test]
fn a_committed_dmabuf_is_counted_after_its_buffer_dies() {
    let _mappings = crate::compositor::dmabuf::tests::exclusive_mappings();
    let mut fixture = start(RendererKind::Pixman, "scoot-cfd-udmabuf");
    for _ in 0..3 {
        if !fixture.done(Step::CommitUdmabuf) {
            return;
        }
    }
    assert_eq!(fixture.state.wl_buffers.buffers_in_flight(), 0);
    assert_eq!(
        fixture.state.client_fds.in_flight(Some(Kind::Plane)),
        3,
        "each surface keeps its plane"
    );
    let client = fixture.client(0).id();
    assert_eq!(fixture.state.client_fds.held_by(&client), 3);
    let files = files_of(&fixture, &client);
    assert_eq!(files.len(), 3, "three udmabufs, three files");
    assert_eq!(
        fds_on(&files),
        3,
        "pixman maps a plane and keeps no fd of its own for it"
    );
    assert_eq!(fixture.state.renderer_plane_copies, None, "no GLES backend");
    fixture.done(Step::DestroySurfaces);
    fixture.settle_until_counted(Kind::Plane, 0);
    assert_eq!(fds_on(&files), 0);
}

/// Multi-plane buffers on the renderer that imports them. A three-plane
/// `YU12` buffer is three client fds, plus whatever copies the renderer
/// keeps of them (one each on the dev VM's llvmpipe, none on a driver that
/// imports into a GEM handle), all counted while a surface keeps the buffer
/// after its `wl_buffer` is destroyed. The count is checked against the
/// dma-buf fds this process really holds, and the loop that would pile them
/// up is refused at the bound -- where the live-buffer count (back at zero
/// every iteration) never would, and where `main` held 1200 dma-buf fds for
/// 200 such buffers.
#[test]
fn multi_plane_dmabufs_count_every_plane_up_to_the_bound() {
    let _flood = hold_flood_lock();
    // Serialised with the suites that map dma-bufs (see
    // `every_advertised_layout_imports_and_draws`). The fds counted below
    // are only those on this test's own buffers' files, so other suites'
    // dma-bufs cannot move them.
    let _mappings = crate::compositor::dmabuf::tests::exclusive_mappings();
    ensure_dispatch_flood_headroom(u64::from(MAX_FDS_PER_CLIENT));
    let mut fixture = start(RendererKind::Gles, "scoot-cfd-yuv");
    let output = fixture.state.outputs.primary_id().expect("an output");
    let imports = fixture.state.backends[&output].imports_dmabuf_format(Format {
        code: Fourcc::Yuv420,
        modifier: Modifier::Linear,
    });
    if !imports {
        eprintln!(
            "multi_plane_dmabufs_count_every_plane_up_to_the_bound: skipped -- this GLES \
             driver does not import YU12 at LINEAR"
        );
        return;
    }
    if !fixture.done(Step::CommitYuv {
        count: 1,
        dups: 0,
        pool: false,
    }) {
        return;
    }
    let copies = u32::from(
        fixture
            .state
            .renderer_plane_copies
            .expect("the first GLES import taught the session its renderer's copies"),
    );
    eprintln!(
        "multi_plane_dmabufs_count_every_plane_up_to_the_bound: {copies} renderer copies per plane"
    );
    let per_buffer = 3 * (1 + copies);
    let client = fixture.client(0).id();
    assert_eq!(fixture.state.wl_buffers.buffers_in_flight(), 0);
    assert_eq!(
        fixture.state.client_fds.in_flight(Some(Kind::Plane)),
        3,
        "one buffer's three planes, kept by the surface"
    );
    assert_eq!(fixture.state.client_fds.held_by(&client), per_buffer);
    assert_eq!(
        fds_on(&files_of(&fixture, &client)),
        per_buffer as usize,
        "the ledger counts every fd the buffer made this process hold"
    );

    // Every buffer whose three adds all fit: each add is admitted at its
    // full weight, so a buffer fits while the buffers before it plus its own
    // whole weight stay within the bound.
    let fit = MAX_FDS_PER_CLIENT / per_buffer;
    assert!(fixture.done(Step::CommitYuv {
        count: fit - 1,
        dups: 0,
        pool: false,
    }));
    assert_eq!(fixture.state.client_fds.held_by(&client), fit * per_buffer);
    let files = files_of(&fixture, &client);
    assert_eq!(
        files.len(),
        fit as usize,
        "one dumb buffer's file per buffer"
    );
    let real = fds_on(&files);
    assert_eq!(real, (fit * per_buffer) as usize);
    assert!(
        real <= MAX_FDS_PER_CLIENT as usize,
        "{real} dma-buf fds held for one client"
    );

    let error = fixture.run_expecting_disconnect(Step::CommitYuv {
        count: 64,
        dups: 0,
        pool: false,
    });
    assert!(
        error.contains(&format!("after {fit} buffers held"))
            && error.contains(&format!("code {NO_MEMORY} on wl_display"))
            && error.contains("zwp_linux_buffer_params_v1.add refused")
            && error.contains(&format!("the maximum is {MAX_FDS_PER_CLIENT}")),
        "{error}"
    );
    fixture.settle_until_counted(Kind::Plane, 0);
    let _ = fixture.render();
    fixture.settle();
    assert_eq!(
        fds_on(&files),
        0,
        "the killed client's planes and copies all closed"
    );
}

/// The renderer probe measures what the import itself opened, not how many
/// fds happen to name the buffer: a client that keeps extra fds on its own
/// buffer's file in this process (here, the in-process client's duplicates;
/// in a real session, fds parked in wayland-backend's received-fd queue)
/// must not teach the session a bigger copy count, which would then charge
/// every other client's planes for copies that do not exist for the rest of
/// the session.
#[test]
fn fds_a_client_parks_on_its_buffer_do_not_skew_the_renderer_probe() {
    let _mappings = crate::compositor::dmabuf::tests::exclusive_mappings();
    let learned = |dups: u32| -> Option<Option<u8>> {
        let mut fixture = start(RendererKind::Gles, "scoot-cfd-probe");
        let output = fixture.state.outputs.primary_id().expect("an output");
        let imports = fixture.state.backends[&output].imports_dmabuf_format(Format {
            code: Fourcc::Yuv420,
            modifier: Modifier::Linear,
        });
        if !imports
            || !fixture.done(Step::CommitYuv {
                count: 1,
                dups,
                pool: false,
            })
        {
            return None;
        }
        Some(fixture.state.renderer_plane_copies)
    };
    let Some(clean) = learned(0) else {
        eprintln!(
            "fds_a_client_parks_on_its_buffer_do_not_skew_the_renderer_probe: skipped -- \
             no YU12 import on this machine"
        );
        return;
    };
    assert!(
        clean.is_some(),
        "the first import taught the session something"
    );
    assert_eq!(
        learned(8),
        Some(clean),
        "eight parked fds on the buffer's file changed what the probe learned"
    );
}

/// A `wl_shm` pool open on the buffer's own file could have fds on that file
/// closed by Smithay's drop thread while the probe counts, which would push
/// the learned number down. Such an import is charged one copy per plane
/// and teaches nothing; the next clean one teaches what a clean session
/// learns. (The race itself is reasoned from the drop thread's code, not
/// reproduced; what this pins is the guard.)
#[test]
fn an_import_a_pool_could_disturb_is_not_learned_from() {
    let _mappings = crate::compositor::dmabuf::tests::exclusive_mappings();
    let mut clean = start(RendererKind::Gles, "scoot-cfd-probe-clean");
    let output = clean.state.outputs.primary_id().expect("an output");
    let imports = clean.state.backends[&output].imports_dmabuf_format(Format {
        code: Fourcc::Yuv420,
        modifier: Modifier::Linear,
    });
    if !imports
        || !clean.done(Step::CommitYuv {
            count: 1,
            dups: 0,
            pool: false,
        })
    {
        eprintln!(
            "an_import_a_pool_could_disturb_is_not_learned_from: skipped -- no YU12 import here"
        );
        return;
    }
    let learned = clean.state.renderer_plane_copies;
    assert!(learned.is_some());
    drop(clean);

    let mut fixture = start(RendererKind::Gles, "scoot-cfd-probe-pool");
    match fixture.run_or_disconnect(Step::CommitYuv {
        count: 1,
        dups: 0,
        pool: true,
    }) {
        Ok(Ack::Done) => {}
        Ok(Ack::NoDevice(reason)) => panic!("the clean fixture had a device: {reason}"),
        Err(error) => {
            eprintln!(
                "an_import_a_pool_could_disturb_is_not_learned_from: skipped -- this machine \
                 will not map a dumb buffer's PRIME fd as a wl_shm pool ({error})"
            );
            return;
        }
    }
    assert_eq!(
        fixture.state.renderer_plane_copies, None,
        "an import a pool on the same file could disturb taught the session nothing"
    );
    let client = fixture.client(0).id();
    assert_eq!(
        fixture.state.client_fds.held_by(&client),
        1 + 3 * 2,
        "the pool, and the three planes charged one copy each"
    );
    fixture.done(Step::CommitYuv {
        count: 1,
        dups: 0,
        pool: false,
    });
    assert_eq!(
        fixture.state.renderer_plane_copies, learned,
        "the next clean import taught what a clean session learns"
    );
}

/// The review of PR #239's shape: a renderer copy is part of what a plane
/// costs, so it has to be inside the bound when the plane is admitted, not
/// added at import after the bound was checked. Fill with committed `YU12`
/// buffers to near the bound, then add planes to params objects (each fits
/// on its own weight of one), then import them all: before the fix all 30
/// adds were admitted and the imports took the client to 540 with nothing
/// refused. Now each `add` weighs its copies too, so the add that would pass
/// 512 is refused, and the weight never passes it.
#[test]
fn renderer_copies_are_inside_the_bound_when_a_plane_is_admitted() {
    let _flood = hold_flood_lock();
    let _mappings = crate::compositor::dmabuf::tests::exclusive_mappings();
    ensure_dispatch_flood_headroom(u64::from(MAX_FDS_PER_CLIENT));
    let mut fixture = start(RendererKind::Gles, "scoot-cfd-overshoot");
    let output = fixture.state.outputs.primary_id().expect("an output");
    let imports = fixture.state.backends[&output].imports_dmabuf_format(Format {
        code: Fourcc::Yuv420,
        modifier: Modifier::Linear,
    });
    if !imports
        || !fixture.done(Step::CommitYuv {
            count: 1,
            dups: 0,
            pool: false,
        })
    {
        eprintln!("renderer_copies_are_inside_the_bound_when_a_plane_is_admitted: skipped");
        return;
    }
    let copies = u32::from(fixture.state.renderer_plane_copies.expect("learned"));
    if copies == 0 {
        eprintln!(
            "renderer_copies_are_inside_the_bound_when_a_plane_is_admitted: skipped -- this \
             renderer keeps no copies, so there is nothing to overshoot with"
        );
        return;
    }
    let per_buffer = 3 * (1 + copies);
    let committed = (MAX_FDS_PER_CLIENT - 3 * 10) / per_buffer;
    assert!(fixture.done(Step::CommitYuv {
        count: committed - 1,
        dups: 0,
        pool: false,
    }));
    let client = fixture.client(0).id();
    let held = committed * per_buffer;
    assert_eq!(fixture.state.client_fds.held_by(&client), held);
    // Each add now weighs 1 + copies. Complete params objects fit while
    // `held + 3n * (1 + copies) <= 512`; the add that would pass it is
    // refused.
    let fit_params = (MAX_FDS_PER_CLIENT - held) / per_buffer;
    let outcome = fixture.run_or_disconnect(Step::HoldYuvParams { params: 10 });
    let error = match outcome {
        Ok(_) => {
            // The old behaviour: every add admitted. Import them and show
            // where that leaves the client.
            let _ = fixture.run_or_disconnect(Step::ImmedHeldParams);
            panic!(
                "all 30 adds were admitted, and after the imports the client holds {} \
                 against a bound of {MAX_FDS_PER_CLIENT}",
                fixture.state.client_fds.held_by(&client)
            );
        }
        Err(error) => error,
    };
    assert!(
        error.contains(&format!("after {fit_params} params complete"))
            && error.contains("zwp_linux_buffer_params_v1.add refused")
            && error.contains(&format!("the maximum is {MAX_FDS_PER_CLIENT}")),
        "{error}"
    );
}

/// How many fds this process holds on any of `files` (`(st_dev, st_ino)`):
/// the planes and every copy a renderer made of them, which share the file,
/// and nothing another test in the same process allocated.
fn fds_on(files: &std::collections::HashSet<(u64, u64)>) -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("/proc/self/fd")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
        .filter(|&fd| {
            // SAFETY: borrowed only for the `fstat`; a number that closed
            // meanwhile fails with `EBADF` and is not counted.
            let fd = unsafe { BorrowedFd::borrow_raw(fd) };
            rustix::fs::fstat(fd).is_ok_and(|stat| {
                #[allow(clippy::useless_conversion)]
                let file = (u64::from(stat.st_dev), u64::from(stat.st_ino));
                files.contains(&file)
            })
        })
        .count()
}

/// The files `client`'s plane records name.
fn files_of(
    fixture: &Fixture,
    client: &smithay::reexports::wayland_server::backend::ClientId,
) -> std::collections::HashSet<(u64, u64)> {
    fixture
        .state
        .client_fds
        .identities_of(client)
        .into_iter()
        .collect()
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestClient {
    synced: bool,
    compositor: Option<wl_compositor::WlCompositor>,
    dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    shm: Option<wl_shm::WlShm>,
}

/// Names memfds uniquely across every client of every test in the process.
static SERIAL: AtomicU32 = AtomicU32::new(0);

fn plane(tag: &str) -> Result<OwnedFd, String> {
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let fd = rustix::fs::memfd_create(format!("{tag} {serial}"), rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|e| e.to_string())?;
    rustix::fs::ftruncate(&fd, 4096).map_err(|e| e.to_string())?;
    Ok(fd)
}

/// `/dev/dri/card0`, as the `drm` crate's device traits want it.
struct Card(std::fs::File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl drm::Device for Card {}
impl drm::control::Device for Card {}

/// One `SIDE`-square `YU12` buffer as a dumb buffer on `card`, exported as a
/// PRIME fd, and its three planes' `(offset, stride)`. The dumb handle is
/// destroyed once exported: the dma-buf holds the memory from then on.
fn yuv_dumb_buffer(card: &Card) -> Result<(OwnedFd, [(u32, u32); 3]), String> {
    let luma = SIDE * SIDE;
    let chroma = (SIDE / 2) * (SIDE / 2);
    let planes = [(0, SIDE), (luma, SIDE / 2), (luma + chroma, SIDE / 2)];
    let rows = (luma + 2 * chroma).div_ceil(256);
    let dumb = card
        .create_dumb_buffer((64, rows), drm::buffer::DrmFourcc::Xrgb8888, 32)
        .map_err(|error| format!("DRM_IOCTL_MODE_CREATE_DUMB: {error}"))?;
    let fd = card.buffer_to_prime_fd(dumb.handle(), drm::CLOEXEC | drm::RDWR);
    let _ = card.destroy_dumb_buffer(dumb);
    let fd = fd.map_err(|error| format!("DRM_IOCTL_PRIME_HANDLE_TO_FD: {error}"))?;
    Ok((fd, planes))
}

fn run_client(
    stream: UnixStream,
    steps: Receiver<Step>,
    acks: Sender<Ack>,
    tag: &'static str,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
    let dmabuf = client.dmabuf.clone().ok_or("no zwp_linux_dmabuf_v1")?;
    let mut surfaces: Vec<wl_surface::WlSurface> = Vec::new();
    let mut pending: Vec<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1> = Vec::new();
    let mut card: Option<Card> = None;
    let mut pools: Vec<wl_shm_pool::WlShmPool> = Vec::new();
    let xr24 = u32::from_ne_bytes(*b"XR24");
    let linear = u64::from(Modifier::Linear);
    while let Ok(step) = steps.recv() {
        let ack = match step {
            Step::AddAndCreate { params, planes } | Step::AddOnly { params, planes } => {
                let create = matches!(step, Step::AddAndCreate { .. });
                for _ in 0..params {
                    let object = dmabuf.create_params(&qh, ());
                    let mut sent = Vec::new();
                    for index in 0..planes {
                        let fd = plane(tag)?;
                        object.add(fd.as_fd(), index, 0, 16, 0, 0);
                        sent.push(fd);
                    }
                    if create {
                        object.create(4, 4, xr24, zwp_linux_buffer_params_v1::Flags::empty());
                        object.destroy();
                    } else {
                        pending.push(object);
                    }
                    sync(&conn, &mut queue, &mut client)?;
                    drop(sent);
                }
                Ack::Done
            }
            Step::CreateAll => {
                for object in pending.drain(..) {
                    object.create(4, 4, xr24, zwp_linux_buffer_params_v1::Flags::empty());
                    object.destroy();
                }
                Ack::Done
            }
            Step::CommitUdmabuf => {
                let memfd = rustix::fs::memfd_create(
                    "scoot-cfd-udmabuf",
                    rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
                )
                .map_err(|e| e.to_string())?;
                let page = rustix::param::page_size() as u64;
                rustix::fs::ftruncate(&memfd, page).map_err(|e| e.to_string())?;
                rustix::fs::fcntl_add_seals(&memfd, rustix::fs::SealFlags::SHRINK)
                    .map_err(|e| e.to_string())?;
                match crate::compositor::dmabuf::tests::udmabuf(memfd.as_fd(), page) {
                    None => Ack::NoDevice("no usable /dev/udmabuf".into()),
                    Some(fd) => {
                        let object = dmabuf.create_params(&qh, ());
                        object.add(fd.as_fd(), 0, 0, 16, 0, 0);
                        let ar24 = u32::from_ne_bytes(*b"AR24");
                        commit_immed(&compositor, &qh, &mut surfaces, object, (4, 4), ar24);
                        sync(&conn, &mut queue, &mut client)?;
                        Ack::Done
                    }
                }
            }
            Step::CommitYuv { count, dups, pool } => {
                if card.is_none() {
                    card = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open("/dev/dri/card0")
                        .ok()
                        .map(Card);
                }
                match card.as_ref() {
                    None => Ack::NoDevice("no usable /dev/dri/card0".into()),
                    Some(card) => {
                        let mut ack = Ack::Done;
                        for _ in 0..count {
                            let (fd, planes) = match yuv_dumb_buffer(card) {
                                Ok(buffer) => buffer,
                                Err(reason) => {
                                    ack = Ack::NoDevice(reason);
                                    break;
                                }
                            };
                            let object = dmabuf.create_params(&qh, ());
                            for (index, (offset, stride)) in planes.into_iter().enumerate() {
                                object.add(
                                    fd.as_fd(),
                                    index as u32,
                                    offset,
                                    stride,
                                    (linear >> 32) as u32,
                                    linear as u32,
                                );
                            }
                            // The adds are where a refusal lands, so they get
                            // their own round trip.
                            let held = surfaces.len();
                            sync(&conn, &mut queue, &mut client)
                                .map_err(|error| format!("after {held} buffers held: {error}"))?;
                            if pool {
                                let shm = client.shm.clone().ok_or("no wl_shm")?;
                                pools.push(shm.create_pool(fd.as_fd(), 4096, &qh, ()));
                                sync(&conn, &mut queue, &mut client)?;
                            }
                            let parked = (0..dups)
                                .map(|_| fd.try_clone().map_err(|e| e.to_string()))
                                .collect::<Result<Vec<_>, _>>()?;
                            if dups == 0 {
                                drop(fd);
                            }
                            commit_immed(
                                &compositor,
                                &qh,
                                &mut surfaces,
                                object,
                                (SIDE as i32, SIDE as i32),
                                Fourcc::Yuv420 as u32,
                            );
                            sync(&conn, &mut queue, &mut client)?;
                            drop(parked);
                        }
                        ack
                    }
                }
            }
            Step::HoldYuvParams { params } => {
                if card.is_none() {
                    card = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open("/dev/dri/card0")
                        .ok()
                        .map(Card);
                }
                let card = card.as_ref().ok_or("no usable /dev/dri/card0")?;
                for n in 0..params {
                    let (fd, planes) = yuv_dumb_buffer(card)?;
                    let object = dmabuf.create_params(&qh, ());
                    for (index, (offset, stride)) in planes.into_iter().enumerate() {
                        object.add(
                            fd.as_fd(),
                            index as u32,
                            offset,
                            stride,
                            (linear >> 32) as u32,
                            linear as u32,
                        );
                    }
                    pending.push(object);
                    sync(&conn, &mut queue, &mut client)
                        .map_err(|error| format!("after {n} params complete: {error}"))?;
                }
                Ack::Done
            }
            Step::ImmedHeldParams => {
                for object in pending.drain(..) {
                    commit_immed(
                        &compositor,
                        &qh,
                        &mut surfaces,
                        object,
                        (SIDE as i32, SIDE as i32),
                        Fourcc::Yuv420 as u32,
                    );
                }
                Ack::Done
            }
            Step::DestroySurfaces => {
                for surface in surfaces.drain(..) {
                    surface.destroy();
                }
                Ack::Done
            }
        };
        sync(&conn, &mut queue, &mut client)?;
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `create_immed` on `object`, attach and commit the buffer on a fresh
/// surface kept in `surfaces`, then destroy the buffer and the params: the
/// surface is all that keeps the planes from here.
fn commit_immed(
    compositor: &wl_compositor::WlCompositor,
    qh: &QueueHandle<TestClient>,
    surfaces: &mut Vec<wl_surface::WlSurface>,
    object: zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
    (width, height): (i32, i32),
    format: u32,
) {
    let buffer = object.create_immed(
        width,
        height,
        format,
        zwp_linux_buffer_params_v1::Flags::empty(),
        qh,
        (),
    );
    object.destroy();
    let surface = compositor.create_surface(qh, ());
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    buffer.destroy();
    surfaces.push(surface);
}

/// A round trip that cannot lose a protocol error to `EPIPE`, the same
/// shape (and for the same race) as `drm_syncobj/tests.rs`'s `sync`.
fn sync(
    conn: &Connection,
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
) -> Result<(), String> {
    client.synced = false;
    conn.display().sync(&queue.handle(), ());
    loop {
        match conn.flush() {
            Ok(()) => break,
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => {
                let _ = queue.dispatch_pending(client);
                if let Some(guard) = conn.prepare_read() {
                    let _ = guard.read();
                }
                let _ = queue.dispatch_pending(client);
                return Err(why(conn, error));
            }
        }
    }
    while !client.synced {
        queue.blocking_dispatch(client).map_err(|e| why(conn, e))?;
    }
    Ok(())
}

fn why(conn: &Connection, error: impl std::fmt::Display) -> String {
    match conn.protocol_error() {
        Some(error) => format!(
            "protocol error code {} on {}@{}: {}",
            error.code, error.object_interface, error.object_id, error.message
        ),
        None => error.to_string(),
    }
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    client.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "zwp_linux_dmabuf_v1" => {
                    client.dmabuf = Some(registry.bind(name, version.min(3), qh, ()));
                }
                "wl_shm" => client.shm = Some(registry.bind(name, version.min(1), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        client.synced = true;
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        _: zwp_linux_buffer_params_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(TestClient, zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, [
        zwp_linux_buffer_params_v1::EVT_CREATED_OPCODE => (wl_buffer::WlBuffer, ()),
    ]);
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _: <zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(TestClient: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(TestClient: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
