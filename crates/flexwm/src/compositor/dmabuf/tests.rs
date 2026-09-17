//! Tests for the `zwp_linux_dmabuf_v1` advertisement (see `dmabuf.rs`).
//!
//! Every one of these drives a *real* `wayland-client` connection through a
//! real [`State`](crate::compositor::State): the client binds the global,
//! reads real feedback events, and performs a real `create_params` import.
//! That last part is the point. The claim under test is "an import attempt is
//! answered `failed` rather than hung or crashed" -- and, for garbage input,
//! "a protocol error kills that client and no one else". Both are claims
//! about what arrived on the client's wire, which a compositor-side assertion
//! cannot make.
//!
//! Like the other real-client suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`](crate::compositor::State::new) binds a
//! real wayland listening socket.
//!
//! No backend, deliberately: [`Harness::bare`](crate::compositor::test_support::Harness::bare)
//! renders nothing, and nothing here needs pixels -- the advertisement is
//! bind-time state, not per-frame work.

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use smithay::backend::allocator::Fourcc;
use smithay::reexports::wayland_server::protocol::wl_shm as server_shm;
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, DispatchError, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};

use super::{DMABUF_FORMATS, main_device, main_device_from};
use crate::compositor::screencopy::FORMATS;
use crate::compositor::test_support::{Harness, wait_for};

/// One instruction for the client thread.
enum Step {
    /// List every global the registry announced, with its version.
    ReportGlobals,
    /// Bind the dmabuf global at `version`, ask for default feedback where
    /// the version has it, and report what arrived.
    ReadFeedback { version: u32 },
    /// Perform a full `create_params` import of a `size`x`size` buffer and
    /// report whether it was created or failed.
    Import {
        format: u32,
        modifier: u64,
        size: i32,
    },
    /// The same with a format the compositor never advertised, reporting the
    /// protocol error that kills this client -- which the client thread
    /// survives over its out-of-band ack channel, so the test can assert the
    /// compositor is still serving everyone else afterwards.
    ImportUnknownFormat,
}

/// What a client answers a [`Step`] with.
enum Ack {
    Globals(Vec<(String, u32)>),
    Feedback(FeedbackSeen),
    Import(ImportOutcome),
    ImportError { code: u32, interface: String },
}

/// The default-feedback events a client saw, as the client saw them.
#[derive(Clone, Debug, Default)]
struct FeedbackSeen {
    done: bool,
    /// The `main_device` bytes, exactly as sent.
    main_device: Vec<u8>,
    /// The format table, parsed into `(fourcc, modifier)` pairs in wire
    /// order -- the order is part of what this compositor promises (opaque
    /// first, like the shm formats).
    table: Vec<(u32, u64)>,
    /// How many tranches were closed by `tranche_done`.
    tranches: u32,
    /// The `format` events a pre-feedback (v1-v3) bind gets instead of any
    /// of the above.
    legacy_formats: Vec<u32>,
}

/// Whether a `create_params` import produced a buffer, was refused, or is
/// still unanswered after the client gave the compositor its chance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ImportOutcome {
    #[default]
    Waiting,
    Created,
    Failed,
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn start() -> Self {
        let mut fixture = Harness::bare(crate::compositor::decorations::Appearance::default());
        fixture.spawn(run_client);
        fixture
    }
}

impl Ack {
    fn globals(self) -> Vec<(String, u32)> {
        match self {
            Ack::Globals(globals) => globals,
            _ => panic!("expected a global list"),
        }
    }

    fn feedback(self) -> FeedbackSeen {
        match self {
            Ack::Feedback(seen) => seen,
            _ => panic!("expected feedback"),
        }
    }

    fn import(self) -> ImportOutcome {
        match self {
            Ack::Import(outcome) => outcome,
            _ => panic!("expected an import outcome"),
        }
    }

    fn import_error(self) -> (u32, String) {
        match self {
            Ack::ImportError { code, interface } => (code, interface),
            _ => panic!("expected a protocol error report"),
        }
    }
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[test]
fn the_dmabuf_global_is_advertised_at_version_6() {
    let mut fixture = Fixture::start();
    let globals = fixture.run(Step::ReportGlobals).globals();
    let version = globals
        .iter()
        .find_map(|(interface, version)| (interface == "zwp_linux_dmabuf_v1").then_some(*version));
    assert_eq!(
        version,
        Some(6),
        "the dmabuf global must be advertised, at the version Smithay's \
         create_global_with_default_feedback fixes (feedback needs v4+; \
         quickshell binds v5, mesa v4, both served from this one global)"
    );
}

#[test]
fn default_feedback_names_a_device_and_both_shm_formats() {
    let mut fixture = Fixture::start();
    // v5: quickshell's own bind version, per the probe record.
    let seen = fixture.run(Step::ReadFeedback { version: 5 }).feedback();
    assert!(seen.done, "the feedback batch must be closed by `done`");
    assert_eq!(
        seen.main_device.len(),
        8,
        "main_device is a dev_t, eight bytes on the wire"
    );
    assert_eq!(
        seen.table,
        vec![
            (u32::from_ne_bytes(*b"XR24"), 0),
            (u32::from_ne_bytes(*b"AR24"), 0),
        ],
        "the table is exactly the two shm formats this compositor serves, \
         opaque first, LINEAR (modifier 0)"
    );
    assert_eq!(seen.tranches, 1, "one tranche carries both formats");
}

#[test]
fn a_v1_client_gets_format_events_not_feedback() {
    let mut fixture = Fixture::start();
    let seen = fixture.run(Step::ReadFeedback { version: 1 }).feedback();
    assert!(
        !seen.done,
        "a v1 bind has no feedback object and therefore no `done`"
    );
    assert!(
        seen.table.is_empty(),
        "a v1 bind has no feedback object and therefore no format table"
    );
    assert_eq!(
        seen.legacy_formats,
        vec![u32::from_ne_bytes(*b"XR24"), u32::from_ne_bytes(*b"AR24"),],
        "a v1 client is told the same two formats through the deprecated events"
    );
}

#[test]
fn a_dmabuf_import_is_answered_failed_not_hung() {
    let mut fixture = Fixture::start();
    let outcome = fixture
        .run(Step::Import {
            format: u32::from_ne_bytes(*b"XR24"),
            modifier: 0, // LINEAR
            size: 32,
        })
        .import();
    assert_eq!(
        outcome,
        ImportOutcome::Failed,
        "this compositor has no dmabuf path, so a well-formed import must be \
         answered `failed` -- the protocol's own refusal -- rather than left \
         unanswered (a hang) or taking the compositor down"
    );
}

#[test]
fn a_garbage_modifier_is_still_answered_failed() {
    let mut fixture = Fixture::start();
    let outcome = fixture
        .run(Step::Import {
            format: u32::from_ne_bytes(*b"XR24"),
            // No advertised tranche carries this; Smithay does not validate
            // the modifier against the table, so it still reaches
            // dmabuf_imported.
            modifier: 0x00DE_ADBE_EF00_0000,
            size: 32,
        })
        .import();
    assert_eq!(
        outcome,
        ImportOutcome::Failed,
        "an unknown modifier must not panic the compositor -- it is `failed` \
         like any other import this compositor cannot honour"
    );
}

#[test]
fn a_garbage_format_kills_only_the_client_that_sent_it() {
    let mut fixture = Fixture::start();
    let other = fixture.spawn(run_client);
    // 0xFFFFFFFF is not a Fourcc at all: Smithay posts InvalidFormat on the
    // params object, which kills this client -- over its out-of-band ack
    // channel the client thread lives to report which error that was.
    let (code, interface) = fixture.run(Step::ImportUnknownFormat).import_error();
    assert_eq!(
        code, 4,
        "an unknown format is InvalidFormat on the params object, not a \
         `failed` event"
    );
    assert_eq!(
        interface, "zwp_linux_buffer_params_v1",
        "the error was posted on the wrong object"
    );
    // The compositor is still up and still advertising: the kill landed on
    // the offending client alone.
    let globals = fixture.run_on(other, Step::ReportGlobals).globals();
    assert!(
        globals
            .iter()
            .any(|(interface, _)| interface == "zwp_linux_dmabuf_v1"),
        "the survivor still sees the dmabuf global after its neighbour was killed"
    );
}

#[test]
fn the_feedback_table_matches_the_shm_capture_formats() {
    // `DMABUF_FORMATS` is deliberately not derived from `screencopy`'s list
    // (no format mapping to get wrong), so this test is the pin that keeps
    // the two lists saying the same thing in the same order. The mapping is
    // explicit rather than numeric: `wl_shm` numbering (`Xrgb8888 = 1`) is
    // not fourcc numbering (`XR24`), so a cast would compare two different
    // schemes that happen to share variant names.
    let shm: Vec<Fourcc> = FORMATS
        .iter()
        .map(|format| match format {
            server_shm::Format::Xrgb8888 => Fourcc::Xrgb8888,
            server_shm::Format::Argb8888 => Fourcc::Argb8888,
            other => panic!("shm capture serves {other:?}, which the dmabuf table does not name"),
        })
        .collect();
    assert_eq!(
        shm.as_slice(),
        &DMABUF_FORMATS,
        "the dmabuf feedback table must name exactly the formats the shm \
         capture path serves, in the same order"
    );
    assert_eq!(
        shm.as_slice(),
        &[Fourcc::Xrgb8888, Fourcc::Argb8888],
        "and those are Xrgb8888 first, Argb8888 second -- see screencopy's \
         module doc for why the opaque one comes first"
    );
}

#[test]
fn the_no_drm_node_ladder_ends_at_zero() {
    let missing = std::path::PathBuf::from("/nonexistent-flexwm-test/card0");
    let also_missing = std::path::PathBuf::from("/nonexistent-flexwm-test/renderD128");
    assert_eq!(
        main_device_from(&missing, &also_missing),
        (0, "no DRM node"),
        "with no DRM node at all -- plausibly the production shape on \
         GPU-less containers -- the ladder degrades to the protocol's own \
         \"no device\" answer"
    );
    // ...while the real ladder never panics, whatever this machine has.
    let _ = main_device();
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TestClient {
    registry: Option<wl_registry::WlRegistry>,
    /// Every global announced, in arrival order.
    globals: Vec<(String, u32)>,
    /// The dmabuf global's name and advertised version, for per-step binds.
    dmabuf_name: Option<(u32, u32)>,
    feedback: FeedbackSeen,
    import: ImportOutcome,
}

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient {
        registry: Some(conn.display().get_registry(&qh, ())),
        ..TestClient::default()
    };
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    // A second round trip for globals advertised after the first batch: the
    // registry is the one object whose announcements can arrive late.
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        match step {
            Step::ReportGlobals => {
                let mut globals = std::mem::take(&mut client.globals);
                globals.sort();
                acks.send(Ack::Globals(globals))
                    .map_err(|e| e.to_string())?;
            }
            Step::ReadFeedback { version } => {
                let (name, advertised) = client.dmabuf_name.ok_or("no zwp_linux_dmabuf_v1")?;
                let registry = client.registry.clone().ok_or("no registry")?;
                client.feedback = FeedbackSeen::default();
                // A fresh bind at the requested version: the test client
                // binds once per step rather than once per connection, so
                // the v1 and v5 shapes are two real binds, not one bind
                // described twice.
                let dmabuf: zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 =
                    registry.bind(name, version.min(advertised), &qh, ());
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                if version >= 4 {
                    let feedback = dmabuf.get_default_feedback(&qh, ());
                    wait_for(&mut queue, &mut client, "default feedback", |client| {
                        client.feedback.done.then_some(())
                    })?;
                    drop(feedback);
                } else {
                    // A pre-feedback bind already received the deprecated
                    // format events during the bind round trips above; one
                    // more round trip to be sure nothing else is coming.
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                }
                dmabuf.destroy();
                acks.send(Ack::Feedback(std::mem::take(&mut client.feedback)))
                    .map_err(|e| e.to_string())?;
            }
            Step::Import {
                format,
                modifier,
                size,
            } => {
                let outcome = import(&qh, &mut queue, &mut client, format, modifier, size)
                    .map_err(|e| e.to_string())?;
                acks.send(Ack::Import(outcome)).map_err(|e| e.to_string())?;
            }
            Step::ImportUnknownFormat => {
                // 0xFFFFFFFF is not a Fourcc at all: Smithay posts
                // InvalidFormat on the params object, which kills this
                // client's *connection* -- but the thread itself lives on
                // over its out-of-band ack channel to report which error
                // that was.
                match import(&qh, &mut queue, &mut client, 0xFFFF_FFFF, 0, 32) {
                    Ok(_) => {
                        acks.send(Ack::Import(client.import))
                            .map_err(|e| e.to_string())?;
                    }
                    Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
                        acks.send(Ack::ImportError {
                            code: error.code,
                            interface: error.object_interface,
                        })
                        .map_err(|e| e.to_string())?;
                        return Ok(());
                    }
                    Err(other) => return Err(other.to_string()),
                }
            }
        }
    }
    Ok(())
}

/// A full `create_params` import: one plane over a real memfd, then `create`,
/// waiting for `created` or `failed`.
///
/// Returns [`ImportOutcome::Waiting`] only if neither arrives -- the hang
/// this suite exists to catch. Returns the client's dispatch error when the
/// compositor kills the connection instead (the garbage-format shape).
fn import(
    qh: &QueueHandle<TestClient>,
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    format: u32,
    modifier: u64,
    size: i32,
) -> Result<ImportOutcome, DispatchError> {
    let (name, advertised) = client.dmabuf_name.expect("no zwp_linux_dmabuf_v1");
    let registry = client.registry.clone().expect("no registry");
    // The advertised version itself (6): the feedback test binds v5 and v1,
    // mesa binds v4 live, so the import path takes the top.
    let dmabuf: zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 = registry.bind(name, advertised, qh, ());
    let params = dmabuf.create_params(qh, ImportSlot);
    let stride = size * 4;
    let fd = rustix::fs::memfd_create("flexwm-dmabuf-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    rustix::fs::ftruncate(&fd, (stride * size) as u64).expect("a sized memfd");
    params.add(
        fd.as_fd(),
        0,
        0,
        stride as u32,
        (modifier >> 32) as u32,
        modifier as u32,
    );
    client.import = ImportOutcome::Waiting;
    params.create(
        size,
        size,
        format,
        zwp_linux_buffer_params_v1::Flags::empty(),
    );
    // `create` is answered synchronously out of request dispatch -- no frame
    // tick involved -- so round trips that flush the requests and read the
    // answer are the whole wait. Anything still `Waiting` afterwards is a
    // compositor that never answered.
    queue.roundtrip(client)?;
    params.destroy();
    dmabuf.destroy();
    Ok(client.import)
}

/// Which import a params object's events report into. One slot: no test here
/// holds two imports open at once.
#[derive(Clone, Copy, Debug)]
struct ImportSlot;

impl Dispatch<wl_registry::WlRegistry, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
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
        if interface == "zwp_linux_dmabuf_v1" {
            client.dmabuf_name = Some((name, version));
        }
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        event: zwp_linux_dmabuf_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The deprecated pre-feedback events, the only thing a v1-v3 bind
        // ever receives. (v4+ binds get these too when Smithay replays the
        // main tranche -- harmless here, since each step binds fresh and
        // reads only what it asked about.)
        match event {
            zwp_linux_dmabuf_v1::Event::Format { format } => {
                client.feedback.legacy_formats.push(format);
            }
            zwp_linux_dmabuf_v1::Event::Modifier { .. } => {}
            _ => {}
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
        match event {
            zwp_linux_dmabuf_feedback_v1::Event::Done => client.feedback.done = true,
            zwp_linux_dmabuf_feedback_v1::Event::MainDevice { device } => {
                client.feedback.main_device = device;
            }
            zwp_linux_dmabuf_feedback_v1::Event::FormatTable { fd, size } => {
                client.feedback.table = read_format_table(fd, size);
            }
            zwp_linux_dmabuf_feedback_v1::Event::TrancheDone => client.feedback.tranches += 1,
            zwp_linux_dmabuf_feedback_v1::Event::TrancheTargetDevice { .. } => {}
            zwp_linux_dmabuf_feedback_v1::Event::TrancheFormats { .. } => {}
            zwp_linux_dmabuf_feedback_v1::Event::TrancheFlags { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, ImportSlot> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
        event: zwp_linux_buffer_params_v1::Event,
        _: &ImportSlot,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `Created` carries the new `wl_buffer`, which this client never
        // initialises: the compositor never sends it (see `dmabuf.rs`), so
        // a regression to answering success fails here -- loudly, in the
        // client's event dispatch -- rather than passing silently.
        match event {
            zwp_linux_buffer_params_v1::Event::Created { .. } => {
                client.import = ImportOutcome::Created;
            }
            zwp_linux_buffer_params_v1::Event::Failed => client.import = ImportOutcome::Failed,
            _ => {}
        }
    }
}

/// The feedback format table, read straight out of the fd Smithay sends:
/// 16-byte entries of `(fourcc, pad, modifier)` in native endian.
fn read_format_table(fd: std::os::fd::OwnedFd, size: u32) -> Vec<(u32, u64)> {
    use std::io::Read;
    let mut file = std::fs::File::from(fd);
    let mut bytes = vec![0u8; size as usize];
    // A short read here is a malformed table, not a retryable condition --
    // but a failure must not panic the client thread either: an empty table
    // fails the assertion downstream, where the failure belongs.
    if file.read_exact(&mut bytes).is_err() {
        return Vec::new();
    }
    let _ = file.as_fd();
    bytes
        .chunks_exact(16)
        .map(|entry| {
            (
                u32::from_ne_bytes(entry[0..4].try_into().unwrap()),
                u64::from_ne_bytes(entry[8..16].try_into().unwrap()),
            )
        })
        .collect()
}
