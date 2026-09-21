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
//! machines that can. Read [`skipped`] before relying on a green run from an
//! unfamiliar machine: libtest captures a passing test's output, so the
//! skip line is only visible under `--nocapture`, and nine of these tests
//! check nothing without that device.
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
use wayland_client::{
    Connection, Dispatch, DispatchError, Proxy, QueueHandle, event_created_child,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1, zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};

use super::{DMABUF_CANDIDATES, imports_linear, main_device, main_device_from, tranche};
use crate::cli::RendererKind;
use crate::compositor::decorations::Appearance;
use crate::compositor::screencopy::FORMATS;
use crate::compositor::test_support::{Harness, wait_for};

/// The headless framebuffer these tests render into. Only the drain test
/// actually draws; it needs a real `PixmanRenderer` behind `State::backends`,
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
    /// Create a `wl_surface`, optionally attach the last imported buffer to
    /// it, commit once, and report the surface's protocol id -- keeping both
    /// alive. The measurement test resolves that id server-side so it can
    /// time the commit-path work directly instead of through round trips.
    MakeSurface { attach_imported: bool },
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
    Surface(u32),
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
    /// A compositor with a real `PixmanRenderer` behind `State::backends` --
    /// which is what makes an import possible at all.
    fn start() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    /// A compositor with no backend, for the one test about what a
    /// renderer-less session advertises.
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

    /// Which renderer this fixture's backend really *built*, which is not the
    /// same question as which one was asked for (see `Backend::renderer`).
    /// The advertisement is derived from the built one, so a test that pins a
    /// renderer-specific answer has to branch on the built one too.
    fn renderer(&self) -> RendererKind {
        let id = self
            .state
            .outputs
            .primary_id()
            .expect("an output behind the fixture");
        self.state
            .backends
            .get(&id)
            .expect("a headless backend behind the fixture")
            .renderer()
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

    fn surface(self) -> u32 {
        match self {
            Ack::Surface(id) => id,
            _ => panic!("expected a surface id"),
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
fn default_feedback_names_a_device_and_the_renderers_own_formats() {
    let mut fixture = Fixture::start();
    let expected = expected_table(&fixture);
    // v5: quickshell's own bind version, per the probe record.
    let seen = fixture.run(Step::ReadFeedback { version: 5 }).feedback();
    assert!(seen.done, "the feedback batch must be closed by `done`");
    assert_eq!(
        seen.main_device.len(),
        8,
        "main_device is a dev_t, eight bytes on the wire"
    );
    assert_eq!(
        seen.table, expected,
        "the table on the wire must be exactly what the session's own renderer \
         can import, in candidate order, LINEAR (modifier 0)"
    );
    assert_eq!(seen.tranches, 1, "one tranche carries the whole table");
    // Under the default renderer the derivation has a known answer, so pin the
    // literal bytes too rather than only the derived ones -- this is the
    // assertion the pixman-only version of this test used to make, and it is
    // what would catch the table's *encoding* going wrong (a swapped pair, a
    // modifier that is not LINEAR) rather than only its contents.
    if fixture.renderer() == RendererKind::Pixman {
        assert_eq!(
            seen.table,
            vec![
                (u32::from_ne_bytes(*b"XR24"), 0),
                (u32::from_ne_bytes(*b"AR24"), 0),
            ],
            "pixman imports both candidates, so its session advertises exactly \
             them, opaque first, LINEAR (modifier 0)"
        );
    }
}

#[test]
fn a_v1_client_gets_format_events_not_feedback() {
    let mut fixture = Fixture::start();
    let expected: Vec<u32> = expected_table(&fixture)
        .into_iter()
        .map(|(code, _modifier)| code)
        .collect();
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
        seen.legacy_formats, expected,
        "a v1 client is told the same formats through the deprecated events"
    );
}

#[test]
fn every_advertised_format_is_one_the_renderer_imports() {
    // The promise with teeth, pinned where the promise is actually made: on
    // the wire, against the renderer this session really built. A
    // `create_immed` the compositor then refuses is a client kill, so nothing
    // a client can read out of the feedback table may be something the
    // importer would turn down.
    //
    // This replaced a pixman-only version of the same pin. It has to be
    // per-renderer now that the table is derived from the active one (see
    // `dmabuf.rs`): under `SCOOT_TEST_RENDERER=gles` it asserts against
    // `GlesRenderer`'s EGL display, which the old one could not see at all.
    //
    // It asks `imports_linear`, the same rule `tranche` filters by, rather
    // than `imports_dmabuf_format({code, Linear})` directly -- and that is
    // not the test weakening itself to match the code. A renderer whose
    // import set carries only `{code, Invalid}` still imports a linear
    // dma-buf of that code (see `imports_linear`'s doc for the chain through
    // the pinned rev), so the direct check is *wrong* about such a driver in
    // the direction that matters: it would fail this test on a session that
    // is behaving correctly.
    let mut fixture = Fixture::start();
    let seen = fixture.run(Step::ReadFeedback { version: 5 }).feedback();
    assert!(
        !seen.table.is_empty(),
        "this renderer advertised nothing at all, so this test would assert \
         nothing -- on a machine whose renderer really can import neither \
         candidate that is the correct behaviour, and this assertion is how \
         you find out that is where you are"
    );
    let output = fixture
        .state
        .outputs
        .primary_id()
        .expect("an output behind the fixture");
    let backend = fixture
        .state
        .backends
        .get(&output)
        .expect("a headless backend behind the fixture");
    for (code, modifier) in seen.table {
        let code = Fourcc::try_from(code).expect("an advertised fourcc is a real one");
        assert_eq!(
            Modifier::from(modifier),
            Modifier::Linear,
            "only LINEAR is ever advertised, whatever the evidence for it was"
        );
        assert!(
            imports_linear(code, &|format| backend.imports_dmabuf_format(format)),
            "the feedback table advertises {code:?} at LINEAR, which this \
             session's renderer will not import -- a client that allocates it \
             and calls create_immed would be killed for believing the \
             advertisement"
        );
    }
}

#[test]
fn pixmans_own_importable_set_still_contains_both_candidates() {
    // The other half of the pin above, and the one that would otherwise be
    // lost: the wire test asserts the table is *honest*, this asserts it is
    // not *empty for the default renderer*. A Smithay bump that dropped
    // `Xrgb8888` or `Argb8888` from pixman's `SUPPORTED_FORMATS` would not
    // fail the wire test at all -- scoot would quietly advertise one format,
    // or none -- and every `wl_shm`-less GL client on the project's own
    // default configuration would silently lose its dmabuf path.
    let renderer = PixmanRenderer::new().expect("a pixman renderer");
    let importable = ImportDma::dmabuf_formats(&renderer);
    for code in DMABUF_CANDIDATES {
        let format = Format {
            code,
            modifier: Modifier::Linear,
        };
        assert!(
            importable.contains(&format),
            "pixman, the default renderer, can no longer import {format:?}, so \
             scoot would stop advertising it"
        );
    }
    // The converse is deliberately *not* asserted: pixman imports far more
    // fourccs than these two, and advertising fewer than can be imported
    // costs a client nothing (it falls back to shm), while advertising one
    // that cannot is fatal. See `DMABUF_CANDIDATES`.
    assert!(
        importable.iter().count() > DMABUF_CANDIDATES.len(),
        "this test only makes sense while pixman's importable set is the \
         larger one; if it ever shrinks to exactly the advertised pair, say \
         so here rather than leaving a vacuous assertion"
    );
}

#[test]
fn a_renderer_that_imports_nothing_is_advertised_as_nothing() {
    // The hazard this whole stage exists for, and the one no machine here can
    // produce on demand: an EGL display with no dma-buf import capability at
    // all. Advertising the candidates against it would kill every client that
    // believed the feedback, so the tranche has to come out empty -- which is
    // what makes `advertise` skip the global entirely rather than offer an
    // empty table.
    let advertised: Vec<Format> = tranche(|_| false).collect();
    assert!(
        advertised.is_empty(),
        "a renderer that can import nothing must be advertised as importing \
         nothing"
    );
}

#[test]
fn a_renderer_missing_one_candidate_advertises_only_the_other() {
    // The partial case, which is the realistic one: a driver that imports
    // opaque `XR24` but not `AR24`. The survivor keeps its place in the
    // candidate order rather than the list being rebuilt in some other one --
    // the order is load-bearing (`Xrgb8888` first; see `screencopy.rs`).
    let opaque = Format {
        code: Fourcc::Xrgb8888,
        modifier: Modifier::Linear,
    };
    let advertised: Vec<Format> = tranche(|format| format == opaque).collect();
    assert_eq!(advertised, vec![opaque]);

    let alpha = Format {
        code: Fourcc::Argb8888,
        modifier: Modifier::Linear,
    };
    let advertised: Vec<Format> = tranche(|format| format == alpha).collect();
    assert_eq!(advertised, vec![alpha]);

    let both: Vec<Format> = tranche(|_| true).collect();
    assert_eq!(
        both,
        vec![opaque, alpha],
        "a renderer that imports both is advertised both, opaque first"
    );
}

#[test]
fn a_renderer_listing_only_the_invalid_modifier_still_advertises_linear() {
    // The regression review caught in PR #147 before it could reach anyone,
    // and the reason `imports_linear` exists rather than a direct
    // `has_dmabuf_format({code, Linear})`.
    //
    // Smithay inserts `{fourcc, Invalid}` unconditionally and explicit
    // modifiers only when `QueryDmaBufModifiersEXT` returned a non-zero count
    // -- which stays zero on a display with no modifiers extension, on a
    // driver that answers `EGL_BAD_PARAMETER` for its own enumerated format
    // (NVIDIA >= 520, named in upstream's own comment), and on a driver that
    // reports no modifiers. Such a renderer *does* import a linear dma-buf.
    // Requiring the explicit entry would have advertised nothing at all
    // there, dropping every GL client to software rendering and leaving a
    // dmabuf-gated shell unable to capture the screen.
    let invalid_only = |format: Format| format.modifier == Modifier::Invalid;
    let advertised: Vec<Format> = tranche(invalid_only).collect();
    assert_eq!(
        advertised,
        vec![
            Format {
                code: Fourcc::Xrgb8888,
                modifier: Modifier::Linear,
            },
            Format {
                code: Fourcc::Argb8888,
                modifier: Modifier::Linear,
            },
        ],
        "a renderer that lists only the Invalid modifier must still be \
         advertised both candidates, and advertised them at LINEAR -- what is \
         on the wire is what a client allocates, and it is never `Invalid`"
    );
}

#[test]
fn a_renderer_with_only_other_explicit_modifiers_advertises_nothing() {
    // The other half of the same rule, and what keeps the widening above from
    // becoming "advertise anything". Evidence is `Linear` *or* `Invalid` and
    // nothing else: a driver that imports a tiled layout for these fourccs
    // and neither of those two says nothing about whether it would take the
    // linear buffer a client allocates from this table -- and a tiled buffer
    // is one pixman cannot map and no consumer of scoot's framebuffer layout
    // expects.
    let tiled = Modifier::from(1u64); // I915_FORMAT_MOD_X_TILED, as a stand-in
    let advertised: Vec<Format> = tranche(|format| format.modifier == tiled).collect();
    assert!(
        advertised.is_empty(),
        "only LINEAR or Invalid is evidence, so a renderer that imports \
         neither must be advertised nothing"
    );
}

/// The `(fourcc, modifier)` pairs this fixture's *own* renderer should put on
/// the wire, derived the same way `advertise` derives them.
///
/// Not a second copy of the expected answer: the point of comparing this
/// against the wire is that the global really carries what the derivation
/// produced, in order, through Smithay's format-table memfd -- a step with
/// several ways to lose the ordering or the modifier and none to notice it.
///
/// This needs no widening of its own for the `Modifier::Invalid` case
/// `imports_linear` handles, and that is worth stating rather than leaving as
/// an absence: the rule lives *inside* `tranche`, so the closure here is
/// called once per candidate per modifier `tranche` considers evidence, and
/// the backend answers each honestly. A copy of the rule here would be a
/// second place for it to drift.
fn expected_table(fixture: &Fixture) -> Vec<(u32, u64)> {
    let output = fixture
        .state
        .outputs
        .primary_id()
        .expect("an output behind the fixture");
    let backend = fixture
        .state
        .backends
        .get(&output)
        .expect("a headless backend behind the fixture");
    tranche(|format| backend.imports_dmabuf_format(format))
        .map(|format| (format.code as u32, u64::from(format.modifier)))
        .collect()
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
        &DMABUF_CANDIDATES,
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
    let missing = std::path::PathBuf::from("/nonexistent-scoot-test/renderD128");
    assert_eq!(
        main_device_from(&missing, &card0).1,
        "/dev/dri/card0",
        "with no render node the primary node is still better than nothing"
    );
}

#[test]
fn the_no_drm_node_ladder_ends_at_zero() {
    let missing = std::path::PathBuf::from("/nonexistent-scoot-test/renderD128");
    let also_missing = std::path::PathBuf::from("/nonexistent-scoot-test/card0");
    assert_eq!(
        main_device_from(&missing, &also_missing),
        (0, "no DRM node"),
        "with no DRM node at all -- plausibly the production shape on \
         GPU-less containers -- the ladder degrades to the protocol's own \
         \"no device\" answer"
    );
    // ...while the real ladder never panics, whatever this machine has.
    let _ = main_device(None);
}

#[test]
fn the_renderers_own_device_beats_the_path_ladder() {
    // The rung the path ladder cannot check: a renderer that names its own
    // EGL device is naming the only device an import can succeed against, so
    // nothing below it is consulted -- including `/dev/dri/renderD128`, which
    // on a two-GPU machine may be the *other* card's node.
    //
    // `0xdead_beef` stands in for a real `dev_t`: nothing here stats it, and
    // using a value no machine can have is what makes a fallthrough to the
    // ladder visible instead of coincidentally equal.
    assert_eq!(
        main_device(Some(0xdead_beef)),
        0xdead_beef,
        "a renderer that names its own device must be believed over any path"
    );
    // And a renderer with no device of its own (pixman, or software EGL)
    // falls through to exactly what the ladder says about this machine.
    assert_eq!(
        main_device(None),
        main_device_from(
            std::path::Path::new("/dev/dri/renderD128"),
            std::path::Path::new("/dev/dri/card0")
        )
        .0,
        "with no renderer device the path ladder is the whole answer"
    );
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
    // ...and the buffer object exists and is counted. This pair is the
    // mechanical guard on Smithay's error-before-init shape that
    // `wl_buffers.rs`'s exactness argument leans on -- but it is the *second*
    // assertion that discriminates, not the first. The `1` below is claimed by
    // `dispatch.rs`'s guard on the `CreateImmed` request itself, so it would
    // be there whether or not Smithay ever initialised the object; only the
    // return to `0` after `ReleaseImportedBuffer` proves a real server-side
    // `wl_buffer` existed for the destruction hook to fire on. Both halves
    // have to be asserted on the *accepted* path: a refused import now
    // releases its own unit (`refuse_import`), so a refused `create_immed`
    // lands on zero whether or not the object was ever initialised and cannot
    // tell the two apart.
    assert_eq!(
        fixture.buffers_in_flight(),
        1,
        "the immed buffer must be claimed against the cap like any other \
         creation -- the init-before-import proof is the release below"
    );
    fixture.run(Step::ReleaseImportedBuffer).released();
    assert_eq!(
        fixture.buffers_in_flight(),
        0,
        "and released by the wl_buffer destruction hook like any other kind"
    );
}

#[test]
fn an_imported_mapping_is_released_without_any_frame() {
    let _mappings = exclusive_mappings();
    // The item's real leak risk, in the shape that actually reaches it.
    // `PixmanRenderer::import_dmabuf` pushes every mapping into a cache that
    // outlives the `wl_buffer`, and `PixmanRenderer::cleanup` -- the only
    // thing that drops an expired entry -- is reached from `Renderer::render`
    // and from `cleanup_texture_cache`, nothing else. Destroying a buffer
    // causes no damage, `render_output` skips `Renderer::render` when there is
    // no damage, and `frame_tick` drops the timer once nothing needs a frame:
    // so "it drains on the next frame" is not a guarantee, and there may be no
    // next frame at all. Hence **no `fixture.render()` anywhere in this
    // test** -- the drain has to happen without one, from the `wl_buffer`
    // destruction hook's queued idle (`dmabuf::schedule_cache_drain`).
    let mut fixture = Fixture::start();
    let before = dmabuf_mappings();
    let outcome = fixture
        .run(Step::Import(Import::new(Backing::Udmabuf)))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("an_imported_mapping_is_released_without_any_frame");
    }
    assert_eq!(outcome, ImportOutcome::Created);
    let mapped = dmabuf_mappings();
    assert!(
        mapped > before,
        "importing a dmabuf must actually map it ({before} mappings before, \
         {mapped} after) -- otherwise this test cannot see the cache at all"
    );

    fixture.run(Step::ReleaseImportedBuffer).released();
    assert_eq!(
        dmabuf_mappings(),
        before,
        "once the client's wl_buffer and fds are gone the mapping must be \
         dropped with no frame in between -- a compositor sitting idle is \
         exactly when no frame is coming, and an mmap held for the process \
         lifetime is the leak this item is about"
    );
}

#[test]
fn repeated_import_and_release_without_a_frame_does_not_grow_the_cache() {
    let _mappings = exclusive_mappings();
    // The bypass shape spelled out: import, destroy, repeat, never commit
    // anything. The per-client live-buffer cap cannot see this -- the count
    // returns to zero on every iteration -- so if the mapping cache did not
    // drain by itself, this loop would pin one dma-buf's pages per pass for
    // the process lifetime. Sixteen passes is enough to distinguish "drains"
    // from "grows"; the assertion is per-iteration so a failure names the
    // pass it first grew on.
    let mut fixture = Fixture::start();
    let before = dmabuf_mappings();
    for pass in 0..16 {
        let outcome = fixture
            .run(Step::Import(Import::new(Backing::Udmabuf)))
            .import();
        if outcome == ImportOutcome::NoUdmabuf {
            return skipped("repeated_import_and_release_without_a_frame_does_not_grow_the_cache");
        }
        assert_eq!(outcome, ImportOutcome::Created, "pass {pass}");
        fixture.run(Step::ReleaseImportedBuffer).released();
        assert_eq!(
            dmabuf_mappings(),
            before,
            "pass {pass}: the mapping cache grew across an import/destroy \
             cycle with no frame rendered -- unbounded pinned memory from a \
             client that has released everything it holds"
        );
        assert_eq!(
            fixture.buffers_in_flight(),
            0,
            "pass {pass}: and the live-buffer count is back to zero, which is \
             why that cap cannot bound the cache"
        );
    }
}

#[test]
fn a_rendered_frame_also_drains_the_mapping_cache() {
    let _mappings = exclusive_mappings();
    // The other half, kept because it is a different mechanism rather than a
    // weaker version of the one above: `Renderer::render` calls
    // `PixmanRenderer::cleanup` on entry (`pixman/mod.rs:866`), which scoot
    // reaches through `OutputDamageTracker::render_output`. That path is real
    // and worth pinning -- it is just not sufficient on its own, which is what
    // the two tests above establish.
    let mut fixture = Fixture::start();
    let before = dmabuf_mappings();
    let outcome = fixture
        .run(Step::Import(Import::new(Backing::Udmabuf)))
        .import();
    if outcome == ImportOutcome::NoUdmabuf {
        return skipped("a_rendered_frame_also_drains_the_mapping_cache");
    }
    assert_eq!(outcome, ImportOutcome::Created);
    assert!(dmabuf_mappings() > before);
    fixture
        .run(Step::CommitImportedBuffer { repeats: 1 })
        .committed();
    fixture.run(Step::ReleaseImportedBuffer).released();
    let _ = fixture.render();
    assert_eq!(
        dmabuf_mappings(),
        before,
        "a frame drawn after the buffer went away must leave no mapping behind"
    );
}

#[test]
fn an_imported_dmabuf_survives_being_recommitted_frame_after_frame() {
    // What a GL client actually does: render into one dmabuf, commit, repeat.
    // Nothing upstream re-synchronises the cached mapping on a re-commit
    // (`PixmanRenderer::existing_dmabuf` hands back the first import`s image
    // untouched), so scoot issues the DMA_BUF_IOCTL_SYNC bracket itself from
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
fn a_session_with_no_renderer_advertises_no_dmabuf_global() {
    // The bare harness has `backend: None`. There is no renderer, so there is
    // nothing whose importable formats a tranche could be derived from, and
    // the honest advertisement is none at all -- a client is steered onto
    // `wl_shm` instead of onto a dmabuf path that could only ever answer
    // `failed`.
    //
    // This test used to drive a real import through the global and assert the
    // refusal. That is no longer reachable *through the protocol*: the global
    // is created from `headless::init_named`, which is also what creates the
    // renderer, so a session with one and not the other cannot be spoken to.
    // `dmabuf_imported`'s no-backend branch stays regardless -- it is the
    // difference between a refusal and a panic for any future front-end that
    // has a renderer at startup and loses it -- but the structural guarantee
    // asserted here is the stronger of the two.
    let mut fixture = Fixture::renderer_less();
    let globals = fixture.run(Step::ReportGlobals).globals();
    assert!(
        !globals
            .iter()
            .any(|(interface, _)| interface == "zwp_linux_dmabuf_v1"),
        "a compositor with no renderer must not advertise a dmabuf global it \
         could only refuse every import on: {globals:?}"
    );
    // ...and it is specifically the dmabuf global that is missing, not the
    // whole session: everything else is still advertised.
    assert!(
        globals
            .iter()
            .any(|(interface, _)| interface == "wl_compositor"),
        "the rest of the session must be unaffected: {globals:?}"
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

/// Prints what the per-commit dmabuf sync costs, for the record CLAUDE.md
/// asks for whenever a change lands on a per-event path. Asserts nothing --
/// a wall-clock threshold in CI is a flake, not a guarantee -- so it is
/// `#[ignore]`d like `cursor/shapes/tests.rs`'s shape dump and run by hand:
///
/// ```text
/// cargo test -p scoot --bin scoot commit_sync_cost -- --ignored --nocapture
/// ```
///
/// Three numbers, because three different clients pay three different prices
/// once any client in the session imports a dmabuf (the gate is session-wide
/// -- see `State::imports_dmabufs`):
///
/// - **gate off** -- every commit in a session with no dmabuf client: one
///   bool test in `commit`, and this function is not called at all.
/// - **no buffer attached** -- the floor for an innocent client once the gate
///   is armed: `is_sync_subsurface`, the `with_surface_tree_downward` walk,
///   and per node a `data_map` type lookup plus a mutex acquire, then an
///   early return. An `wl_shm` client pays this plus one failed downcast
///   (`get_dmabuf` is `buffer.data::<Dmabuf>()`, a pointer compare), so this
///   is that client's cost to within noise.
/// - **dmabuf attached** -- the client the work is for: the above plus two
///   `DMA_BUF_IOCTL_SYNC` calls, one of which waits on the buffer's fences.
#[test]
#[ignore = "prints per-commit timings for a human; asserts nothing"]
fn commit_sync_cost() {
    const ROUNDS: u32 = 20_000;
    let _mappings = exclusive_mappings();
    let mut fixture = Fixture::start();
    if fixture
        .run(Step::Import(Import::new(Backing::Udmabuf)))
        .import()
        == ImportOutcome::NoUdmabuf
    {
        return skipped("commit_sync_cost");
    }

    for (label, attach_imported) in [("no buffer attached", false), ("dmabuf attached", true)] {
        let id = fixture.run(Step::MakeSurface { attach_imported }).surface();
        let surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface = fixture
            .client(0)
            .object_from_protocol_id(&fixture.state.display_handle, id)
            .expect("the measured surface");
        // Warm: the first call faults in the mapping and the type-map entry.
        super::sync_committed_dmabufs(&surface);
        let started = std::time::Instant::now();
        for _ in 0..ROUNDS {
            super::sync_committed_dmabufs(&surface);
        }
        let each = started.elapsed() / ROUNDS;
        println!("commit sync, {label}: {each:?} per commit ({ROUNDS} rounds)");
    }
    println!(
        "commit sync, gate off: not called -- one bool test in `commit` \
         (see State::imports_dmabufs)"
    );
}

/// Records that a test asserted nothing because this machine has no usable
/// `/dev/udmabuf`.
///
/// **How visible this actually is, stated rather than assumed:** libtest
/// captures a passing test's stdout and stderr, so on a machine without
/// `/dev/udmabuf` this line appears only under `cargo test -- --nocapture`
/// (or `cargo nextest run --no-capture`), or bundled into the output of some
/// *other* failing test in the same binary. It is not a `#[ignore]` and it
/// does not colour the summary: nine of this suite's tests will report `ok`
/// having checked nothing.
///
/// That is the deliberate trade -- see the module doc -- but it means
/// `/dev/udmabuf` is a prerequisite for this suite meaning anything about
/// import, not an optional extra, and a reviewer confirming the import path
/// on a new machine should check the count of tests that really ran rather
/// than the summary line.
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
    /// Surfaces `Step::MakeSurface` made and deliberately kept alive.
    surfaces: Vec<wl_surface::WlSurface>,
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
            Step::MakeSurface { attach_imported } => {
                let compositor: wl_compositor::WlCompositor = {
                    let (name, version) = client.compositor_name.ok_or("no wl_compositor")?;
                    let registry = client.registry.clone().ok_or("no registry")?;
                    registry.bind(name, version.min(4), &qh, ())
                };
                let surface = compositor.create_surface(&qh, ());
                if attach_imported {
                    let buffer = client.buffer.clone().ok_or("no imported buffer")?;
                    surface.attach(Some(&buffer), 0, 0);
                    surface.damage_buffer(0, 0, IMPORT_SIZE, IMPORT_SIZE);
                }
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let id = surface.id().protocol_id();
                // Kept alive on purpose: the caller times work against this
                // exact surface tree, server-side.
                client.surfaces.push(surface);
                acks.send(Ack::Surface(id)).map_err(|e| e.to_string())?;
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
        // `create_immed` has no `created` event: the buffer object exists the
        // moment the request is sent -- which is why a refused import there is
        // fatal -- and the only thing the compositor can say afterwards is that
        // it failed. So the proxy is kept here rather than waiting for an
        // event (`ReleaseImportedBuffer` destroys whatever is kept), the
        // outcome is optimistic, and only a `failed` event overwrites it.
        client.buffer = Some(params.create_immed(
            IMPORT_SIZE,
            IMPORT_SIZE,
            format,
            zwp_linux_buffer_params_v1::Flags::empty(),
            qh,
            (),
        ));
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
        "scoot-dmabuf-test",
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
