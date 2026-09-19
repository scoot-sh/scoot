//! Two halves, matching what the collection has to get right.
//!
//! The first is [`Outputs`] on its own: ids that start at one and never
//! repeat, and lookups that answer for an output this compositor has and
//! refuse for one it does not.
//!
//! The second is the session `--headless --outputs N` builds, through a real
//! [`State`] with a real backend -- because "two outputs exist" is a claim
//! about the core's rectangles, Smithay's `Space` and the `wl_output` globals
//! a client can actually bind, not about a `Vec`'s length. The wire half is
//! what the multi-output item needs most: if a client cannot address the
//! second output, nothing per-output can be tested against it.

use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use scoot_core::{OutputId, Rect};
use smithay::desktop::layer_map_for_output;
use smithay::output::{Output, PhysicalProperties, Subpixel};
use wayland_client::protocol::{wl_compositor, wl_output, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::Outputs;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::test_support::Harness;

/// The framebuffer the primary output renders into, and the size every
/// `--outputs` output is created at. Square and small: nothing here draws.
const CANVAS: i32 = 200;

fn output_named(name: &str) -> Output {
    Output::new(
        name.to_owned(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "scoot".into(),
            model: name.to_owned(),
            serial_number: "0".into(),
        },
    )
}

// -------------------------------------------------------------------------
// The collection on its own
// -------------------------------------------------------------------------

#[test]
fn an_empty_collection_has_no_primary_and_nothing_to_find() {
    let outputs = Outputs::default();
    assert!(outputs.is_empty());
    assert!(outputs.primary().is_none());
    assert!(outputs.primary_id().is_none());
    assert!(outputs.primary_entry().is_none());
    assert!(outputs.last().is_none());
    assert!(outputs.get(OutputId(1)).is_none());
    assert_eq!(outputs.iter().count(), 0);
}

#[test]
fn ids_start_at_one_and_never_repeat() {
    // One, because a single-output session has always reported `OutputId(1)`
    // over IPC and to the core -- the const this replaced was `OutputId(1)`.
    let mut outputs = Outputs::default();
    let first = outputs.add(output_named("headless"));
    let second = outputs.add(output_named("headless-2"));
    let third = outputs.add(output_named("headless-3"));
    assert_eq!(
        [first, second, third],
        [OutputId(1), OutputId(2), OutputId(3)]
    );
}

#[test]
fn the_primary_output_is_the_first_one_added() {
    // Load-bearing, not incidental: the primary output is the one with the
    // render target, which is what lets a capture of any other output be
    // refused rather than answered from the wrong framebuffer.
    let mut outputs = Outputs::default();
    let first = output_named("headless");
    let second = output_named("headless-2");
    let first_id = outputs.add(first.clone());
    outputs.add(second.clone());
    assert_eq!(outputs.primary(), Some(&first));
    assert_eq!(outputs.primary_id(), Some(first_id));
    assert_eq!(outputs.primary_entry(), Some((first_id, &first)));
    // ...and `last` is the other end, which is where `add_output` measures
    // the next output's position from.
    assert_eq!(outputs.last(), Some(&second));
}

#[test]
fn get_answers_for_an_output_we_have_and_refuses_one_we_do_not() {
    let mut outputs = Outputs::default();
    let first = output_named("headless");
    let second = output_named("headless-2");
    let first_id = outputs.add(first.clone());
    let second_id = outputs.add(second.clone());
    assert_eq!(outputs.get(first_id), Some(&first));
    assert_eq!(outputs.get(second_id), Some(&second));
    // Zero is below the first id ever handed out, and three is above the
    // last: neither names an output, and neither may fall back to one.
    assert!(outputs.get(OutputId(0)).is_none());
    assert!(outputs.get(OutputId(3)).is_none());
}

#[test]
fn iteration_follows_creation_order() {
    let mut outputs = Outputs::default();
    let names = ["headless", "headless-2", "headless-3"];
    for name in names {
        outputs.add(output_named(name));
    }
    let seen: Vec<String> = outputs.iter().map(|output| output.name()).collect();
    assert_eq!(seen, names);
}

// -------------------------------------------------------------------------
// The session `--headless --outputs N` builds
// -------------------------------------------------------------------------

/// One instruction for the client thread.
enum Step {
    /// Report every `wl_output` global the registry announced.
    Outputs,
    /// Create a layer surface naming the `index`-th `wl_output` the registry
    /// announced, commit it without a buffer, and report whether the
    /// compositor answered with the initial configure the protocol owes it.
    CreateLayerOn { index: usize },
    /// `zwlr_layer_surface_v1.destroy` on the layer surface the last
    /// [`Step::CreateLayerOn`] made.
    DestroyLayer,
}

/// What the client reports back.
enum Ack {
    /// [`Step::Outputs`]'s answer: one record per `wl_output`, in the order
    /// the registry announced them.
    Outputs(Vec<Screen>),
    /// [`Step::CreateLayerOn`]'s answer: did the initial configure arrive?
    LayerConfigured(bool),
    Done,
}

/// What a client learns about one output off plain `wl_output` -- no
/// `xdg-output` needed, since v4 carries the name and `geometry` carries the
/// output's position in the global compositor space.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Screen {
    name: Option<String>,
    position: Option<(i32, i32)>,
    mode: Option<(i32, i32)>,
}

/// A bound output's slot in [`TestClient::screens`].
struct ScreenIndex(usize);

#[derive(Default)]
struct TestClient {
    screens: Vec<Screen>,
    /// Held so the globals stay bound and their events keep arriving.
    outputs: Vec<wl_output::WlOutput>,
    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    /// Whether the layer surface under test has been configured, and the
    /// serial to ack -- the whole question of the hang this guards against.
    layer_configure: Option<u32>,
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
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_output" => {
                // Version 4 for `wl_output.name`, which is how a bar tells two
                // screens apart.
                let index = client.screens.len();
                client.screens.push(Screen::default());
                client
                    .outputs
                    .push(registry.bind(name, version.min(4), qh, ScreenIndex(index)));
            }
            "wl_compositor" => {
                client.compositor = Some(registry.bind(name, version.min(6), qh, ()));
            }
            "zwlr_layer_shell_v1" => {
                client.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ScreenIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &ScreenIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(screen) = client.screens.get_mut(index.0) else {
            return;
        };
        match event {
            wl_output::Event::Name { name } => screen.name = Some(name),
            // `geometry`'s x/y is the output's position in the global
            // compositor space -- the whole point of a second output.
            wl_output::Event::Geometry { x, y, .. } => screen.position = Some((x, y)),
            wl_output::Event::Mode { width, height, .. } => screen.mode = Some((width, height)),
            _ => {}
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for TestClient {
    fn event(
        client: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            // Acked at once, the way any real bar does: an unacked configure
            // is not evidence the surface is usable.
            surface.ack_configure(serial);
            client.layer_configure = Some(serial);
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(TestClient: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(TestClient: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

/// A layer surface and its `wl_surface`, held for the test's duration so
/// wayland-client does not destroy a role the compositor still has mapped.
type KeptLayer = (
    wl_surface::WlSurface,
    zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
);

fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    // Twice: the first round trip binds whatever globals the registry
    // announced, the second collects the events those binds produced.
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let mut layers: Vec<KeptLayer> = Vec::new();
    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let outcome = match step {
            Step::Outputs => Ack::Outputs(client.screens.clone()),
            Step::CreateLayerOn { index } => {
                let compositor = client.compositor.clone().ok_or("no wl_compositor")?;
                let shell = client
                    .layer_shell
                    .clone()
                    .ok_or("no zwlr_layer_shell_v1 -- the global is missing")?;
                let output = client
                    .outputs
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("no wl_output at index {index}"))?;
                client.layer_configure = None;
                let surface = compositor.create_surface(&qh, ());
                let layer = shell.get_layer_surface(
                    &surface,
                    Some(&output),
                    zwlr_layer_shell_v1::Layer::Top,
                    "scoot-output-probe".to_owned(),
                    &qh,
                    (),
                );
                // A bar: full width, fixed height, anchored to three edges.
                // A zero width with left+right anchors is the protocol's own
                // "the compositor chooses", so this is a legal description
                // and the configure it earns carries a real size.
                layer.set_size(0, 24);
                layer.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Top
                        | zwlr_layer_surface_v1::Anchor::Left
                        | zwlr_layer_surface_v1::Anchor::Right,
                );
                // No buffer: the initial configure is the answer to the first
                // commit, and a client may not attach before it acks one.
                surface.commit();
                let mut configured = false;
                for _ in 0..50 {
                    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                    if client.layer_configure.is_some() {
                        configured = true;
                        break;
                    }
                }
                layers.push((surface, layer));
                Ack::LayerConfigured(configured)
            }
            Step::DestroyLayer => {
                let (surface, layer) = layers.pop().ok_or("no layer surface to destroy")?;
                layer.destroy();
                surface.destroy();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Done
            }
        };
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with `count` outputs and one connected client.
///
/// The extra outputs are added before the client connects, so the registry
/// announces every one of them in the client's first round trip -- the same
/// order `compositor::run` builds them in.
fn session(count: i32) -> Harness<Step, Ack> {
    let mut harness = Harness::headless(Appearance::default(), CANVAS);
    for index in 2..=count {
        headless::add_output(
            &mut harness.state,
            &format!("{}-{index}", headless::OUTPUT_NAME),
            CANVAS,
            CANVAS,
        )
        .expect("another headless output");
    }
    harness.spawn(run_client);
    harness
}

/// The `wl_output` globals the client can see, in registry order.
fn outputs_of(harness: &mut Harness<Step, Ack>) -> Vec<Screen> {
    match harness.run(Step::Outputs) {
        Ack::Outputs(screens) => screens,
        _ => panic!("expected the output list"),
    }
}

#[test]
fn one_output_is_exactly_the_session_that_shipped_before() {
    // The `N == 1` half of the deliverable: nothing about the default session
    // may have moved. One output, the id every single-output session has
    // always reported, at the origin, at the size it was created with.
    let mut harness = session(1);
    assert_eq!(harness.state.outputs.iter().count(), 1);
    assert_eq!(harness.state.outputs.primary_id(), Some(OutputId(1)));
    assert_eq!(
        harness.state.world.outputs(),
        vec![(OutputId(1), Rect::new(0, 0, CANVAS, CANVAS))]
    );

    let screens = outputs_of(&mut harness);
    assert_eq!(
        screens,
        vec![Screen {
            name: Some(headless::OUTPUT_NAME.to_owned()),
            position: Some((0, 0)),
            mode: Some((CANVAS, CANVAS)),
        }]
    );
}

#[test]
fn a_second_output_gets_its_own_id_its_own_place_and_its_own_wl_output() {
    let mut harness = session(2);

    // The compositor's own side: two outputs, distinct ids, the second one
    // mapped into the `Space` immediately right of the first.
    assert_eq!(harness.state.outputs.iter().count(), 2);
    let names: Vec<String> = harness.state.outputs.iter().map(Output::name).collect();
    assert_eq!(names, vec!["headless", "headless-2"]);
    let second = harness
        .state
        .outputs
        .get(OutputId(2))
        .cloned()
        .expect("the second output");
    let geometry = harness
        .state
        .space
        .output_geometry(&second)
        .expect("the second output is in the space");
    assert_eq!((geometry.loc.x, geometry.loc.y), (CANVAS, 0));
    assert_eq!((geometry.size.w, geometry.size.h), (CANVAS, CANVAS));

    // The core's side: two outputs, adjacent and non-overlapping, each with
    // its own rectangle -- which is what gives each its own scrolling strip.
    assert_eq!(
        harness.state.world.outputs(),
        vec![
            (OutputId(1), Rect::new(0, 0, CANVAS, CANVAS)),
            (OutputId(2), Rect::new(CANVAS, 0, CANVAS, CANVAS)),
        ]
    );

    // The client's side, which is the half that matters for everything
    // per-output that comes next: two `wl_output` globals, distinguishable by
    // name, each reporting its own position.
    let screens = outputs_of(&mut harness);
    assert_eq!(
        screens,
        vec![
            Screen {
                name: Some("headless".to_owned()),
                position: Some((0, 0)),
                mode: Some((CANVAS, CANVAS)),
            },
            Screen {
                name: Some("headless-2".to_owned()),
                position: Some((CANVAS, 0)),
                mode: Some((CANVAS, CANVAS)),
            },
        ]
    );
}

#[test]
fn every_extra_output_is_placed_beside_the_last_one() {
    // Three, so "beside the previous one" is distinguishable from "beside the
    // primary one" -- the third has to land at 2 * CANVAS, not at CANVAS.
    let harness = session(3);
    assert_eq!(
        harness.state.world.outputs(),
        vec![
            (OutputId(1), Rect::new(0, 0, CANVAS, CANVAS)),
            (OutputId(2), Rect::new(CANVAS, 0, CANVAS, CANVAS)),
            (OutputId(3), Rect::new(2 * CANVAS, 0, CANVAS, CANVAS)),
        ]
    );
}

#[test]
fn an_extra_output_is_refused_before_the_primary_one_exists() {
    // Structural, not a matter of call order in `run`: the invariant that the
    // primary output is the one with the render target is what lets every
    // other output's capture be refused instead of quietly answered from the
    // wrong framebuffer.
    let mut harness: Harness<Step, Ack> = Harness::bare(Appearance::default());
    let error = headless::add_output(&mut harness.state, "headless-2", CANVAS, CANVAS)
        .expect_err("an output before the primary one should be refused");
    assert!(
        error.to_string().contains("primary"),
        "the refusal should say what is missing: {error}"
    );
    assert!(harness.state.outputs.is_empty());
}

#[test]
fn only_the_composited_output_can_be_screenshotted() {
    // The wrong-pixels guard. With two outputs and one framebuffer, a capture
    // of the second one has no pixels to answer with, and handing back the
    // first one's would label a picture of one screen as another.
    let harness = session(2);
    assert_eq!(harness.state.screenshot_refusal(None), None);
    assert_eq!(harness.state.screenshot_refusal(Some(1)), None);
    let refusal = harness
        .state
        .screenshot_refusal(Some(2))
        .expect("a capture of the second output is refused");
    assert!(
        refusal.contains("output 2") && refusal.contains("composites one output"),
        "the refusal should name what was asked for and why: {refusal}"
    );
    // An id naming no output at all is refused the same way, with one output
    // or with two.
    assert!(harness.state.screenshot_refusal(Some(99)).is_some());
}

#[test]
fn each_output_reports_its_own_name_over_ipc() {
    // `outputs` used to stamp the single output's name on every entry the
    // core reported. With a real second output that would name the wrong
    // screen, which is what an agent picks a target from.
    let harness = session(2);
    let snapshots = harness.state.output_snapshots();
    let named: Vec<(u64, String)> = snapshots
        .iter()
        .map(|snapshot| (snapshot.id, snapshot.name.clone()))
        .collect();
    assert_eq!(
        named,
        vec![(1, "headless".to_owned()), (2, "headless-2".to_owned())]
    );
    assert_eq!(snapshots[1].rect.x, CANVAS);
}

#[test]
fn a_layer_surface_on_a_secondary_output_is_configured_and_then_unmapped() {
    // The one step here that is more than a lookup swap, and the reason it
    // is: `commit_layer_surface` and `layer_destroyed` used to reach for the
    // single output unconditionally. With a real second output that is two
    // client-facing faults, not an incompleteness -- a commit on a surface
    // the primary's map does not hold reads as "not a layer surface", so no
    // initial configure is ever sent and the client waits forever; and the
    // destruction never unmaps it, leaving a dead surface arranged on an
    // output whose map `render()`'s `cleanup()` does not walk.
    let mut harness = session(2);
    let second = harness
        .state
        .outputs
        .get(OutputId(2))
        .cloned()
        .expect("the second output");

    match harness.run(Step::CreateLayerOn { index: 1 }) {
        Ack::LayerConfigured(true) => {}
        Ack::LayerConfigured(false) => {
            panic!("a layer surface on the second output never got its initial configure")
        }
        _ => panic!("expected the layer answer"),
    }
    assert_eq!(
        layer_map_for_output(&second).layers().count(),
        1,
        "the surface should be mapped on the output its client named"
    );
    assert_eq!(
        layer_map_for_output(harness.state.outputs.primary().expect("the primary output"))
            .layers()
            .count(),
        0,
        "and on no other"
    );

    match harness.run(Step::DestroyLayer) {
        Ack::Done => {}
        _ => panic!("expected the destroy answer"),
    }
    assert_eq!(
        layer_map_for_output(&second).layers().count(),
        0,
        "destroying the role should unmap it from the output that holds it"
    );
}
