//! Tests for `zwp_linux_dmabuf_v1` -- the advertisement *and* the import (see
//! `dmabuf.rs`).
//!
//! Every one of these drives a *real* `wayland-client` connection through a
//! real [`State`](crate::compositor::State): the client binds the global,
//! reads real feedback events, and performs a real `create_params` import.
//! That last part is the point. The claims under test are "a well-formed
//! dmabuf really is imported and the client really is handed its `wl_buffer`",
//! "a buffer this renderer cannot map is refused rather than trusted", and,
//! for garbage input, "a protocol error kills that client and no one else".
//! All three are claims about what arrived on the client's wire, which a
//! compositor-side assertion cannot make.
//!
//! The import tests need a *real* dma-buf, not a stand-in: pixman's
//! `import_dmabuf` issues `DMA_BUF_IOCTL_SYNC` before it maps anything, and
//! that ioctl fails with `ENOTTY` on a plain memfd -- which is itself worth a
//! test ([`Backing::Memfd`]), but means a memfd cannot stand in for the
//! success path. [`Backing::Udmabuf`] makes one through `/dev/udmabuf`, the
//! kernel's own "export this memfd as a dma-buf" device. Where that device is
//! absent or not permitted, those tests report it and pass without asserting
//! -- a machine with no udmabuf cannot answer the question, and pretending
//! otherwise would either fail honest builds or hide the answer on the
//! machines that can.
//!
//! Like the other real-client suites here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`](crate::compositor::State::new) binds a
//! real wayland listening socket.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

use std::sync::mpsc::{Receiver, Sender};

use smithay::backend::allocator::{Format, Fourcc, Modifier};
use smithay::backend::renderer::ImportDma;
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::reexports::wayland_server::protocol::wl_shm as server_shm;
use wayland_client::backend::WaylandError;
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, DispatchError, QueueHandle, event_created_child};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};

use super::{DMABUF_FORMATS, main_device, main_device_from};
use crate::compositor::decorations::Appearance;
use crate::compositor::screencopy::FORMATS;
use crate::compositor::test_support::{Harness, wait_for};

/// The headless framebuffer these tests render into. Only the drain test
/// actually draws; it needs a real `PixmanRenderer` behind `State::backend`,
/// which is what a headless harness (unlike a bare one) provides.
const CANVAS: i32 = 32;

/// The square dmabuf every import step allocates, in pixels. Small on
/// purpose: nothing here reads a pixel, and `/dev/udmabuf` rounds its
/// allocation up to a page anyway.
const IMPORT_SIZE: i32 = 4;

/// One instruction for the client thread.
enum Step {
    /// List every global the registry announced, with its version.
    ReportGlobals,
    /// Bind the dmabuf global at `version`, ask for default feedback where
    /// the version has it, and report what arrived.
    ReadFeedback { version: u32 },
    /// Perform a full `create_params` import and report whether the
    /// compositor created a buffer or refused.
    Import(Import),
    /// The same with a format the compositor never advertised, reporting the
    /// protocol error that kills this client -- which the client thread
    /// survives over its out-of-band ack channel, so the test can assert the
    /// compositor is still serving everyone else afterwards.
    ImportUnknownFormat,
    /// Destroy the `wl_buffer` the last successful [`Step::Import`] produced,
    /// and drop every fd behind it, so nothing but the compositor's own
    /// mapping can still be holding the dma-buf alive.
    ReleaseImportedBuffer,
    /// Attach the `wl_buffer` the last successful [`Step::Import`] produced to
    /// a fresh `wl_surface` and commit it, `repeats` times over -- the shape a
    /// GL client re-rendering into one dmabuf actually has, and the only way
    /// to drive `dmabuf::sync_committed_dmabufs` from a test.
    CommitImportedBuffer { repeats: usize },
}

/// What a [`Step::Import`] offers the compositor.
#[derive(Clone, Copy)]
struct Import {
    backing: Backing,
    format: u32,
    modifier: u64,
    /// How many planes to `add`. One is the only shape pixman imports;
    /// anything else must be refused.
    planes: u32,
    /// Use `create_immed` rather than the asynchronous `create`. The two
    /// differ in what a refusal *means* (a `failed` event vs. a fatal
    /// protocol error), which is the whole reason this item exists.
    immed: bool,
}

impl Import {
    /// A single-plane `LINEAR` `Argb8888` import over `backing`, through the
    /// asynchronous `create` -- what every test here varies from.
    fn new(backing: Backing) -> Self {
        Self {
            backing,
            format: u32::from_ne_bytes(*b"AR24"),
            modifier: 0,
            planes: 1,
            immed: false,
        }
    }
}

/// What the plane fds an [`Import`] sends are really backed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backing {
    /// A plain memfd: enough to satisfy every check Smithay makes, never
    /// enough to import, because `DMA_BUF_IOCTL_SYNC` answers `ENOTTY` on
    /// anything that is not a dma-buf.
    Memfd,
    /// A real dma-buf, exported from a sealed memfd by `/dev/udmabuf`.
    Udmabuf,
}

/// What a client answers a [`Step`] with.
enum Ack {
    Globals(Vec<(String, u32)>),
    Feedback(FeedbackSeen),
    Import(ImportOutcome),
    ImportError { code: u32, interface: String },
    Released,
    Committed,
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

/// Whether a `create_params` import produced a buffer, was refused, is still
/// unanswered after the client gave the compositor its chance, or could not
/// be attempted at all on this machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ImportOutcome {
    #[default]
    Waiting,
    Created,
    Failed,
    /// `/dev/udmabuf` is missing or not permitted here, so no real dma-buf
    /// could be made and the step asserted nothing. See the module doc.
    NoUdmabuf,
}

type Fixture = Harness<Step, Ack>;

impl Fixture {
    /// A compositor with a real `PixmanRenderer` behind `State::backend` --
    /// which is what makes an import possible at all.
    fn start() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    /// A compositor with no backend, for the one test about what a
    /// renderer-less session answers.
    fn renderer_less() -> Self {
        let mut fixture = Harness::bare(Appearance::default());
        fixture.spawn(run_client);
        fixture
    }

    /// How many live `wl_buffer`s the cap believes every client holds
    /// between them -- the bookkeeping the dmabuf factories claim into (see
    /// `wl_buffers.rs`).
    fn buffers_in_flight(&self) -> usize {
        self.state.wl_buffers.buffers_in_flight()
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

    fn released(self) {
        match self {
            Ack::Released => {}
            _ => panic!("expected a release acknowledgement"),
        }
    }

    fn committed(self) {
        match self {
            Ack::Committed => {}
            _ => panic!("expected a commit acknowledgement"),
        }
    }
}

/// How many dma-buf mappings this process holds, read out of
/// `/proc/self/maps`.
///
/// The compositor and its test client share a process here, and only the
/// compositor ever `mmap`s a dma-buf (the client passes fds and never maps
/// them), so this counts exactly the mappings `PixmanRenderer`'s
/// `dmabuf_cache` is holding. The kernel names a dma-buf mapping's backing
/// file `/dmabuf:...` in `maps`, which nothing else in this process does.
///
/// This is the only handle a test has on that cache: `dmabuf_cache` is a
/// private field of a type in another crate, so the mapping it retains is
/// observable only through the kernel.
/// Serialises every test that holds a real dma-buf mapping open.
///
/// [`dmabuf_mappings`] counts a *process-global* quantity, and `cargo test`
/// runs this module's tests as threads of one process -- so without this a
/// neighbour's mapping is indistinguishable from the one under test, which is
/// exactly how the drain assertion first failed here (`before` read 1, not 0,
/// because another test's import was in flight). Held for a whole test rather
/// than around each read: what has to be true is that no other mapping is
/// created *or* destroyed while one test is measuring. Free by construction
/// under `cargo nextest`, which gives each test its own process.
///
/// Taken through poisoning on purpose: the data is `()`, so a panicking
/// neighbour has nothing to corrupt, and turning one failure into eight would
/// hide which test actually broke.
static ONE_MAPPING_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn exclusive_mappings() -> std::sync::MutexGuard<'static, ()> {
    ONE_MAPPING_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn dmabuf_mappings() -> usize {
    std::fs::read_to_string("/proc/self/maps")
        .expect("this platform has /proc/self/maps")
        .lines()
        .filter(|line| line.contains("/dmabuf"))
        .count()
}

// ---------------------------------------------------------------------------
// The advertisement
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
fn default_feedback_names_a_device_and_both_advertised_formats() {
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
        "the table is exactly the two formats this compositor imports and \
         serves, opaque first, LINEAR (modifier 0)"
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
fn every_advertised_format_is_one_pixman_can_import() {
    // The promise with teeth: a `create_immed` the compositor then refuses is
    // a client kill, so nothing may be in the tranche that
    // `PixmanRenderer::import_dmabuf` would turn down. Pinned against the
    // renderer's own `dmabuf_formats()` rather than against a comment, so a
    // Smithay bump that drops a format from `SUPPORTED_FORMATS` fails here
    // instead of in someone's session.
    let renderer = PixmanRenderer::new().expect("a pixman renderer");
    let importable = ImportDma::dmabuf_formats(&renderer);
    for code in DMABUF_FORMATS {
        let format = Format {
            code,
            modifier: Modifier::Linear,
        };
        assert!(
            importable.contains(&format),
            "the feedback table advertises {format:?}, which this renderer \
             cannot import -- a client that allocates it and calls \
             create_immed would be killed for believing the advertisement"
        );
    }
    // The converse is deliberately *not* asserted: pixman imports far more
    // fourccs than these two, and advertising fewer than can be imported
    // costs a client nothing (it falls back to shm), while advertising one
    // that cannot is fatal. See `DMABUF_FORMATS`.
    assert!(
        importable.iter().count() > DMABUF_FORMATS.len(),
        "this test only makes sense while pixman's importable set is the \
         larger one; if it ever shrinks to exactly the advertised pair, say \
         so here rather than leaving a vacuous assertion"
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
fn the_main_device_ladder_prefers_the_render_node() {
    // The device in the feedback is what clients allocate against now that
    // imports are real, and a client that only renders has no business on a
    // primary node. `/dev/null` stands in for "a node that exists": the
    // ladder asks `stat` for an rdev, not DRM for anything.
    let render = std::path::PathBuf::from("/dev/null");
    let card0 = std::path::PathBuf::from("/dev/zero");
    assert_eq!(
        main_device_from(&render, &card0).1,
        "/dev/dri/renderD128",
        "with both nodes present the render node must answer"
    );
    let missing = std::path::PathBuf::from("/nonexistent-flexwm-test/renderD128");
    assert_eq!(
        main_device_from(&missing, &card0).1,
        "/dev/dri/card0",
        "with no render node the primary node is still better than nothing"
    );
}

#[test]
fn the_no_drm_node_ladder_ends_at_zero() {
    let missing = std::path::PathBuf::from("/nonexistent-flexwm-test/renderD128");
    let also_missing = std::path::PathBuf::from("/nonexistent-flexwm-test/card0");
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
// The import
// ---------------------------------------------------------------------------

#[test]
fn a_real_dmabuf_is_imported_and_the_client_gets_its_buffer() {
    let _mappings = exclusive_mappings();
    // The regression this whole item exists for, in its cheapest form: a
    // single-plane LINEAR dmabuf of an advertised format must come back
    // `created`, not `failed`. Before the fix this answered `failed` -- and
    // the same import through `create_immed` killed the client outright,
    // which is what took noctalia v5 down.
    let mut fixture = Fixture::start();
    let outcome = fixture
        .run(Step::Import(Import::new(Backing::Udmabuf)))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("a_real_dmabuf_is_imported_and_the_client_gets_its_buffer");
    }
    assert_eq!(
        outcome,
        ImportOutcome::Created,
        "a well-formed single-plane LINEAR dmabuf of an advertised format \
         must be imported into the pixman renderer and answered with a real \
         wl_buffer"
    );
}

#[test]
fn an_import_through_create_immed_is_not_a_client_kill() {
    let _mappings = exclusive_mappings();
    // The same import the other way round. `create_immed` is the request Mesa
    // actually sends, and it is the one with no soft refusal: a compositor
    // that cannot import posts `InvalidWlBuffer` and the client dies. Passing
    // here means the client survived *and* was told its buffer exists.
    let mut fixture = Fixture::start();
    let outcome = fixture
        .run(Step::Import(Import {
            immed: true,
            ..Import::new(Backing::Udmabuf)
        }))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("an_import_through_create_immed_is_not_a_client_kill");
    }
    assert_eq!(
        outcome,
        ImportOutcome::Created,
        "create_immed must import; a refusal here is a fatal protocol error \
         on the client, not a fallback"
    );
}

#[test]
fn an_imported_mapping_is_released_when_the_buffer_goes_away() {
    let _mappings = exclusive_mappings();
    // The item's real leak risk. `PixmanRenderer::import_dmabuf` pushes every
    // mapping into a cache that outlives the `wl_buffer`, and only
    // `PixmanRenderer::cleanup` -- which `Renderer::render` calls on entry --
    // drops the expired ones. If flexwm never rendered, every dmabuf a client
    // ever committed would keep an mmap and an fd for the process lifetime,
    // and the per-client buffer cap would not bound it.
    let mut fixture = Fixture::start();
    let before = dmabuf_mappings();
    let outcome = fixture
        .run(Step::Import(Import::new(Backing::Udmabuf)))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("an_imported_mapping_is_released_when_the_buffer_goes_away");
    }
    assert_eq!(outcome, ImportOutcome::Created);
    let mapped = dmabuf_mappings();
    assert!(
        mapped > before,
        "importing a dmabuf must actually map it ({before} mappings before, \
         {mapped} after) -- otherwise this test cannot see the cache at all"
    );

    fixture.run(Step::ReleaseImportedBuffer).released();
    // The frame is the drain: `cleanup` runs from `Renderer::render`, which
    // flexwm reaches through `OutputDamageTracker::render_output`.
    let _ = fixture.render();
    assert_eq!(
        dmabuf_mappings(),
        before,
        "once the client's wl_buffer and fds are gone, the next rendered \
         frame must drop the renderer's cached mapping -- a cache that only \
         grows is an mmap and an fd leaked per buffer, for the process \
         lifetime"
    );
}

#[test]
fn an_imported_dmabuf_survives_being_recommitted_frame_after_frame() {
    // What a GL client actually does: render into one dmabuf, commit, repeat.
    // Nothing upstream re-synchronises the cached mapping on a re-commit
    // (`PixmanRenderer::existing_dmabuf` hands back the first import`s image
    // untouched), so flexwm issues the DMA_BUF_IOCTL_SYNC bracket itself from
    // the commit handler -- see `dmabuf::sync_committed_dmabufs`. This drives
    // that path for real: a surface tree walked on every commit, with the
    // sync ioctl landing on a live dma-buf.
    //
    // What it can prove from inside a test is the mechanism, not the pixels:
    // that the walk runs, re-entrant with `on_commit_buffer_handler`s own
    // locks, without deadlocking or panicking, and that the compositor keeps
    // serving the client afterwards. Freshness itself is a hardware claim and
    // is recorded as one.
    let _mappings = exclusive_mappings();
    let mut fixture = Fixture::start();
    let outcome = fixture
        .run(Step::Import(Import::new(Backing::Udmabuf)))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("an_imported_dmabuf_survives_being_recommitted_frame_after_frame");
    }
    assert_eq!(outcome, ImportOutcome::Created);
    assert!(
        fixture.state.imports_dmabufs,
        "a successful import must arm the per-commit sync; without it the 
         commits below walk nothing"
    );

    fixture
        .run(Step::CommitImportedBuffer { repeats: 8 })
        .committed();
    let _ = fixture.render();

    // Still serving: the client was not killed, and the compositor did not
    // wedge on its own surface-state locks.
    let globals = fixture.run(Step::ReportGlobals).globals();
    assert!(
        globals
            .iter()
            .any(|(interface, _)| interface == "zwp_linux_dmabuf_v1"),
        "the client must still be connected after eight dmabuf commits"
    );
}

#[test]
fn an_accepted_import_claims_and_releases_one_buffer_unit() {
    let _mappings = exclusive_mappings();
    // `ImportNotifier::successful` on an async `create` notifier mints a real
    // wl_buffer, so the async path has to claim against
    // MAX_BUFFERS_PER_CLIENT like every other factory -- it did not before
    // this item, because it never created anything.
    let mut fixture = Fixture::start();
    assert_eq!(fixture.buffers_in_flight(), 0);
    let outcome = fixture
        .run(Step::Import(Import::new(Backing::Udmabuf)))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("an_accepted_import_claims_and_releases_one_buffer_unit");
    }
    assert_eq!(outcome, ImportOutcome::Created);
    assert_eq!(
        fixture.buffers_in_flight(),
        1,
        "a buffer minted by the async `create` path must be counted, or a GL \
         client's buffers sit outside the per-client cap entirely"
    );
    fixture.run(Step::ReleaseImportedBuffer).released();
    assert_eq!(
        fixture.buffers_in_flight(),
        0,
        "and released by the same wl_buffer destruction hook as every other \
         kind"
    );
}

#[test]
fn a_refused_async_import_hands_its_buffer_unit_back() {
    // The other half of the claim. `create` is the one creation whose refusal
    // leaves the client alive, so a claim with no matching release would let
    // a client ratchet its own count to the cap just by offering buffers this
    // renderer cannot map -- and then be refused buffers it *could* have had.
    let mut fixture = Fixture::start();
    for _ in 0..4 {
        let outcome = fixture
            .run(Step::Import(Import::new(Backing::Memfd)))
            .import();
        assert_eq!(outcome, ImportOutcome::Failed);
    }
    assert_eq!(
        fixture.buffers_in_flight(),
        0,
        "four refused imports must leave the count where they found it"
    );
}

#[test]
fn a_fake_dmabuf_over_a_plain_memfd_is_refused_not_trusted() {
    // Every check Smithay makes passes (real fd, real size, advertised
    // format, LINEAR, one plane), and the import still must not happen: an
    // fd that is not a dma-buf answers DMA_BUF_IOCTL_SYNC with ENOTTY, and
    // pixman refuses rather than mapping it anyway. The client is told
    // `failed` and lives.
    let mut fixture = Fixture::start();
    let outcome = fixture
        .run(Step::Import(Import::new(Backing::Memfd)))
        .import();
    assert_eq!(
        outcome,
        ImportOutcome::Failed,
        "an fd that is not really a dma-buf must be refused -- not mapped on \
         the client's say-so"
    );
}

#[test]
fn a_garbage_modifier_is_still_answered_failed() {
    let _mappings = exclusive_mappings();
    let mut fixture = Fixture::start();
    let outcome = fixture
        .run(Step::Import(Import {
            // No advertised tranche carries this; Smithay does not validate
            // the modifier against the table, so it still reaches
            // dmabuf_imported -- where pixman refuses anything but LINEAR.
            modifier: 0x00DE_ADBE_EF00_0000,
            ..Import::new(Backing::Udmabuf)
        }))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("a_garbage_modifier_is_still_answered_failed");
    }
    assert_eq!(
        outcome,
        ImportOutcome::Failed,
        "a modifier this renderer cannot read must not panic the compositor \
         and must not be imported -- it is `failed`, the protocol's own \
         refusal"
    );
}

#[test]
fn a_multi_plane_import_is_refused() {
    let _mappings = exclusive_mappings();
    // Pixman maps plane 0 and nothing else, so a multi-plane buffer is
    // refused (`UnsupportedNumberOfPlanes`). The tranche never offers a
    // multi-plane format, so only a client ignoring its own feedback gets
    // here; what matters is that it is a refusal rather than a
    // half-composited surface.
    let mut fixture = Fixture::start();
    let outcome = fixture
        .run(Step::Import(Import {
            planes: 2,
            ..Import::new(Backing::Udmabuf)
        }))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("a_multi_plane_import_is_refused");
    }
    assert_eq!(
        outcome,
        ImportOutcome::Failed,
        "pixman maps plane 0 only, so a multi-plane dmabuf must be refused"
    );
}

#[test]
fn a_session_with_no_renderer_answers_failed() {
    let _mappings = exclusive_mappings();
    // The bare harness has `backend: None`. There is nothing to import into,
    // so the honest answer is the protocol's own refusal -- and, crucially,
    // not a panic on an `unwrap` of a backend that isn't there.
    let mut fixture = Fixture::renderer_less();
    let outcome = fixture
        .run(Step::Import(Import::new(Backing::Udmabuf)))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("a_session_with_no_renderer_answers_failed");
    }
    assert_eq!(
        outcome,
        ImportOutcome::Failed,
        "a compositor with no renderer must refuse, not crash"
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

/// Says, once and loudly enough to read in a test log, that a test asserted
/// nothing because this machine has no usable `/dev/udmabuf`.
///
/// Not a silent pass: a run where these are skipped has not answered the
/// import question at all, and the log line is what says so. See the module
/// doc for why a memfd cannot stand in.
fn skipped(test: &str) {
    eprintln!("{test}: skipped -- no usable /dev/udmabuf on this machine");
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
    /// The same for `wl_compositor`, which only the commit step needs.
    compositor_name: Option<(u32, u32)>,
    feedback: FeedbackSeen,
    import: ImportOutcome,
    /// The `wl_buffer` the last import produced, kept so a later step can
    /// destroy it, and the fds behind it, kept so the compositor's mapping is
    /// provably the last thing holding the dma-buf alive.
    buffer: Option<wl_buffer::WlBuffer>,
    planes: Vec<OwnedFd>,
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
            Step::Import(request) => {
                let outcome = match import(&qh, &mut queue, &mut client, request, request.format) {
                    Ok(outcome) => outcome,
                    Err(error) => return Err(error.to_string()),
                };
                acks.send(Ack::Import(outcome)).map_err(|e| e.to_string())?;
            }
            Step::ImportUnknownFormat => {
                // 0xFFFFFFFF is not a Fourcc at all: Smithay posts
                // InvalidFormat on the params object, which kills this
                // client's *connection* -- but the thread itself lives on
                // over its out-of-band ack channel to report which error
                // that was.
                let request = Import::new(Backing::Memfd);
                match import(&qh, &mut queue, &mut client, request, 0xFFFF_FFFF) {
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
            Step::CommitImportedBuffer { repeats } => {
                let buffer = client.buffer.clone().ok_or("no imported buffer")?;
                let compositor: wl_compositor::WlCompositor = {
                    let (name, version) = client.compositor_name.ok_or("no wl_compositor")?;
                    let registry = client.registry.clone().ok_or("no registry")?;
                    registry.bind(name, version.min(4), &qh, ())
                };
                let surface = compositor.create_surface(&qh, ());
                for _ in 0..repeats {
                    surface.attach(Some(&buffer), 0, 0);
                    surface.damage_buffer(0, 0, IMPORT_SIZE, IMPORT_SIZE);
                    surface.commit();
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                }
                surface.destroy();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Committed).map_err(|e| e.to_string())?;
            }
            Step::ReleaseImportedBuffer => {
                if let Some(buffer) = client.buffer.take() {
                    buffer.destroy();
                }
                // The fds too: the compositor's own `Dmabuf` (in the
                // wl_buffer's user data) has to be the last strong reference
                // for the cache entry to expire, but the *kernel* object also
                // has to go for the mapping count to fall.
                client.planes.clear();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                acks.send(Ack::Released).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// A full `create_params` import, waiting for `created` or `failed`.
///
/// `format` is passed separately from `request.format` so the garbage-format
/// step can send something that is not a fourcc at all while keeping every
/// other parameter well-formed.
///
/// Returns [`ImportOutcome::Waiting`] only if neither answer arrives -- the
/// hang this suite exists to catch -- and [`ImportOutcome::NoUdmabuf`] when
/// the step asked for a real dma-buf this machine cannot make. Returns the
/// client's dispatch error when the compositor kills the connection instead
/// (the garbage-format shape).
fn import(
    qh: &QueueHandle<TestClient>,
    queue: &mut wayland_client::EventQueue<TestClient>,
    client: &mut TestClient,
    request: Import,
    format: u32,
) -> Result<ImportOutcome, DispatchError> {
    let stride = IMPORT_SIZE as u32 * 4;
    client.planes.clear();
    for _ in 0..request.planes {
        let Some(fd) = plane_fd(request.backing, stride * IMPORT_SIZE as u32) else {
            return Ok(ImportOutcome::NoUdmabuf);
        };
        client.planes.push(fd);
    }

    let (name, advertised) = client.dmabuf_name.expect("no zwp_linux_dmabuf_v1");
    let registry = client.registry.clone().expect("no registry");
    // The advertised version itself (6): the feedback test binds v5 and v1,
    // mesa binds v4 live, so the import path takes the top.
    let dmabuf: zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 = registry.bind(name, advertised, qh, ());
    let params = dmabuf.create_params(qh, ImportSlot);
    for (index, fd) in client.planes.iter().enumerate() {
        params.add(
            fd.as_fd(),
            index as u32,
            0,
            stride,
            (request.modifier >> 32) as u32,
            request.modifier as u32,
        );
    }
    client.import = ImportOutcome::Waiting;
    client.buffer = None;
    if request.immed {
        params.create_immed(
            IMPORT_SIZE,
            IMPORT_SIZE,
            format,
            zwp_linux_buffer_params_v1::Flags::empty(),
            qh,
            (),
        );
        // `create_immed` has no `created` event: the buffer exists the moment
        // the request is sent, and the only thing the compositor can say is
        // that it failed. Surviving the round trip below *is* the success
        // signal, so the outcome is set here and only `failed` overwrites it.
        client.import = ImportOutcome::Created;
    } else {
        params.create(
            IMPORT_SIZE,
            IMPORT_SIZE,
            format,
            zwp_linux_buffer_params_v1::Flags::empty(),
        );
    }
    // Both requests are answered synchronously out of request dispatch -- no
    // frame tick involved -- so round trips that flush the requests and read
    // the answer are the whole wait. Anything still `Waiting` afterwards is a
    // compositor that never answered.
    queue.roundtrip(client)?;
    params.destroy();
    dmabuf.destroy();
    Ok(client.import)
}

/// One plane fd of `size` bytes: a plain memfd, or a real dma-buf over
/// `/dev/udmabuf`.
fn plane_fd(backing: Backing, size: u32) -> Option<OwnedFd> {
    let memfd = rustix::fs::memfd_create(
        "flexwm-dmabuf-test",
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .expect("a memfd");
    match backing {
        Backing::Memfd => {
            rustix::fs::ftruncate(&memfd, u64::from(size)).expect("a sized memfd");
            Some(memfd)
        }
        Backing::Udmabuf => {
            // udmabuf works in whole pages and refuses a memfd that can still
            // shrink under it, so the allocation is rounded up and sealed.
            let page = rustix::param::page_size() as u64;
            let bytes = u64::from(size).div_ceil(page) * page;
            rustix::fs::ftruncate(&memfd, bytes).expect("a sized memfd");
            rustix::fs::fcntl_add_seals(&memfd, rustix::fs::SealFlags::SHRINK).ok()?;
            udmabuf(memfd.as_fd(), bytes)
        }
    }
}

/// Exports `memfd` as a real dma-buf through `/dev/udmabuf`, or `None` where
/// this machine has no usable one (absent device, no permission, no
/// `CONFIG_UDMABUF`).
///
/// `libc::ioctl` rather than a `rustix` wrapper because `UDMABUF_CREATE`
/// returns the new fd as the ioctl's own return value, which `rustix`'s typed
/// ioctl helpers have no shape for.
fn udmabuf(memfd: BorrowedFd<'_>, size: u64) -> Option<OwnedFd> {
    /// `_IOW('u', 0x42, struct udmabuf_create)` -- 24 bytes of payload.
    const UDMABUF_CREATE: u32 = 0x4018_7542;

    #[repr(C)]
    struct UdmabufCreate {
        memfd: u32,
        flags: u32,
        offset: u64,
        size: u64,
    }

    let device = rustix::fs::open(
        "/dev/udmabuf",
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let create = UdmabufCreate {
        memfd: memfd.as_raw_fd() as u32,
        // UDMABUF_FLAGS_CLOEXEC: the fd this hands back must not leak into a
        // child of the test process.
        flags: 0x01,
        offset: 0,
        size,
    };
    // SAFETY: `device` is an open fd for the whole call, `create` is a
    // correctly-shaped `struct udmabuf_create` living across it, and the
    // driver only reads it. A non-negative return is a fresh owned fd, which
    // is what `OwnedFd` is then given; a negative one is an errno and owns
    // nothing.
    let fd = unsafe {
        libc::ioctl(
            device.as_raw_fd(),
            UDMABUF_CREATE as _,
            std::ptr::addr_of!(create),
        )
    };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
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
        if interface == "wl_compositor" {
            client.compositor_name = Some((name, version));
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
        match event {
            // `Created` carries the new `wl_buffer`, which the client keeps so
            // a later step can destroy it -- the drain test needs the
            // compositor's mapping to be the last thing holding the dma-buf.
            zwp_linux_buffer_params_v1::Event::Created { buffer } => {
                client.import = ImportOutcome::Created;
                client.buffer = Some(buffer);
            }
            zwp_linux_buffer_params_v1::Event::Failed => client.import = ImportOutcome::Failed,
            _ => {}
        }
    }

    event_created_child!(TestClient, zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1, [
        zwp_linux_buffer_params_v1::EVT_CREATED_OPCODE => (wl_buffer::WlBuffer, ()),
    ]);
}

impl Dispatch<wl_compositor::WlCompositor, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `wl_compositor` has no events.
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `enter`/`leave`/`preferred_buffer_*`: nothing this suite asserts on.
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `release` is the only event, and nothing here attaches a buffer to
        // a surface, so it never arrives.
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
