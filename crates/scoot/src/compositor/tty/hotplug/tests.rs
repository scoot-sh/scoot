//! What a fresh probe of the DRM device *means*, decided without one.
//!
//! [`super::plan`] is the only part of the *decision* that can be exercised
//! off real hardware: everything around it -- `gpu::reselect`,
//! `DrmSurface::use_mode`/`set_connectors`, `BufferPool::new` -- needs a
//! live DRM file descriptor, and `drm`'s two device traits are blanket
//! implementations over `AsFd` with no seam to fake (the same reason
//! `gpu.rs`'s own tests stop at device *ordering* and never reach
//! `find_connector_and_mode`). Those paths are covered on the dev VM
//! instead; see the PR for what was run there.
//!
//! What *is* covered here beyond [`super::plan`] is
//! [`super::Reconfigured::finish`] for the CRTC-switch outcome: it needs no
//! DRM device, only a headless [`State`](crate::compositor::State), and it is
//! where the switch meets everything downstream (the render target via
//! `resize_output`, `wl_output` clients, and `zwlr_gamma_control_v1`). The
//! surface rebuild itself (`switch_crtc`) needs a multi-CRTC device no
//! hardware here has, and stays stated-as-unverified like the ticket.
//!
//! Connector handles are real ones, built from the `NonZeroU32` the kernel
//! identifies a connector by, because that is exactly what
//! `resources.connectors()` hands back -- nothing here is a stand-in type.
//!
//! Like the other client-driven suites here, the `finish` tests need a
//! writable `$XDG_RUNTIME_DIR`.

use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender};

use smithay::reexports::drm::control::connector;
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1, zwlr_gamma_control_v1,
};

use super::{Plan, Reconfigured, plan};
use crate::compositor::decorations::Appearance;
use crate::compositor::gamma_control::FALLBACK_GAMMA_SIZE;
use crate::compositor::test_support::{Harness, wait_for};

/// The connector the kernel would call id `raw`.
fn connector(raw: u32) -> connector::Handle {
    connector::Handle::from(NonZeroU32::new(raw).expect("connector ids start at 1"))
}

const EDP: u32 = 71;
const HDMI: u32 = 82;
const FHD: (i32, i32) = (1920, 1080);
const SMALL: (i32, i32) = (848, 480);

#[test]
fn the_same_connector_at_the_same_size_is_not_a_change() {
    // The common case by far: a `change` uevent fires for anything the
    // kernel considers a change to the card, and most of them have nothing
    // to do with which connector is lit or how big it is. Tearing the
    // display down for one would mean a visible blank every time something
    // unrelated twitched.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(EDP), FHD))),
        Plan::Unchanged
    );
}

#[test]
fn the_same_connector_at_a_new_size_is_a_mode_change() {
    // Issue #48's first case: the vfkit window moved between a 2x and a 1x
    // screen, so virtio-gpu's `Virtual-1` now offers -- and prefers -- a
    // different size, on the same connector it always had.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(EDP), SMALL))),
        Plan::NewMode
    );
}

#[test]
fn a_different_connector_is_a_connector_change_even_at_the_same_size() {
    // Issue #48's second case, in its awkward variant: the laptop panel was
    // unplugged (or went away) and an external monitor of *exactly* the
    // same resolution took over. Nothing about the size says anything
    // happened, but the surface still has to be moved onto the new
    // connector or it keeps driving one that is gone.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(HDMI), FHD))),
        Plan::NewConnector
    );
}

#[test]
fn a_different_connector_at_a_different_size_is_still_one_connector_change() {
    // Both changed at once, which is the normal shape of the unplug case.
    // It must not read as a mode change on the old connector -- the mode
    // belongs to the new one, and applying it without moving the surface
    // first is what `set_pending` exists to get right.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(HDMI), SMALL))),
        Plan::NewConnector
    );
}

#[test]
fn nothing_connected_is_its_own_answer_not_a_change_to_apply() {
    // The case that used to be a black screen until restart. It is
    // deliberately *not* folded into any of the three above: there is no
    // connector to move to and no mode to set, so the only thing to do is
    // hold the last frame and say so.
    assert_eq!(plan((connector(EDP), FHD), None), Plan::NoConnector);
}

#[test]
fn only_the_width_changing_is_still_a_mode_change() {
    // Guards the tuple comparison against being written as a comparison of
    // one dimension: an ultrawide re-probing from 2560x1080 to 3440x1080
    // keeps its height and is absolutely a new mode.
    assert_eq!(
        plan(
            (connector(EDP), (2560, 1080)),
            Some((connector(EDP), (3440, 1080)))
        ),
        Plan::NewMode
    );
}

#[test]
fn only_the_height_changing_is_still_a_mode_change() {
    assert_eq!(
        plan((connector(EDP), (1920, 1200)), Some((connector(EDP), FHD))),
        Plan::NewMode
    );
}

#[test]
fn a_zero_sized_probe_is_treated_as_a_change_like_any_other() {
    // Not reachable through `gpu::connector_mode` -- the kernel does not
    // list a 0x0 mode -- but `plan` must not be the thing that decides it
    // cannot happen. Reporting it as a change routes it into `retarget`,
    // where a real `BufferPool::new`/`use_mode` rejects it with a logged
    // error and the display stays as it was; silently reporting `Unchanged`
    // for a nonsense probe would hide that.
    assert_eq!(
        plan((connector(EDP), FHD), Some((connector(EDP), (0, 0)))),
        Plan::NewMode
    );
}

// -- `Reconfigured::SwitchedCrtc::finish` --------------------------------------
// A CRTC switch lands through `finish` like every other reconfigure outcome,
// and that is where it is pinned: the size half is the same `resize_output`
// the `Resized` arm runs (so `wl_output` must agree with it), and the gamma
// half is `crtc_changed` (so a later `get_gamma_control` must report the new
// CRTC's length, and a live control must hear `failed` only when the length
// actually moved).

/// The framebuffer the fixtures render into. Nothing here reads a pixel; the
/// backend exists so the compositor has a real output, as it does in a session.
const CANVAS: i32 = 200;

/// A second size to switch to, for the size-changing tests. Different from
/// [`CANVAS`] in both dimensions so a partially-applied resize cannot pass.
const BIG: i32 = 300;

/// A plausible other CRTC's LUT length: a real size, and not the fallback, so
/// a test can tell "re-read from the new CRTC" apart from "never updated".
const NEW_GAMMA_SIZE: u32 = 1024;

/// One `wl_output.mode` event, as the client saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SeenMode {
    width: i32,
    height: i32,
    current: bool,
    preferred: bool,
}

/// One instruction for the client thread: bind `wl_output` (the gamma
/// manager's registry name is recorded on the way in), create another gamma
/// control and report the `gamma_size` it arrives with, or hand back what
/// `wl_output` / the live controls have said since the last such step.
enum Step {
    Bind,
    GetGamma,
    TakeModes,
    TakeFailed,
}

enum Ack {
    Done,
    GammaSize(u32),
    Modes(Vec<SeenMode>),
    Failed(Vec<bool>),
}

#[derive(Default)]
struct TestClient {
    manager_name: Option<(u32, u32)>,
    output_name: Option<(u32, u32)>,
    controls: Vec<zwlr_gamma_control_v1::ZwlrGammaControlV1>,
    /// One slot per control created, in creation order: the last `gamma_size`
    /// seen on it, and whether it has been told `failed`.
    sizes: Vec<Option<u32>>,
    failed: Vec<bool>,
    /// Every `wl_output.mode` event since the last [`Step::TakeModes`].
    modes: Vec<SeenMode>,
}

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
        match interface.as_str() {
            "zwlr_gamma_control_manager_v1" => client.manager_name = Some((name, version)),
            "wl_output" => client.output_name = Some((name, version)),
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let wl_output::Event::Mode {
            flags,
            width,
            height,
            ..
        } = event
        else {
            return;
        };
        let (current, preferred) = flags.into_result().map_or((false, false), |flags| {
            (
                flags.contains(wl_output::Mode::Current),
                flags.contains(wl_output::Mode::Preferred),
            )
        });
        client.modes.push(SeenMode {
            width,
            height,
            current,
            preferred,
        });
    }
}

impl Dispatch<zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1, ()> for TestClient {
    fn event(
        _: &mut Self,
        _: &zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
        _: zwlr_gamma_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Which control in creation order an event belongs to: `get_gamma_control`
/// hands over the object first and its `gamma_size` after.
struct ControlIndex(usize);

impl Dispatch<zwlr_gamma_control_v1::ZwlrGammaControlV1, ControlIndex> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwlr_gamma_control_v1::ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        index: &ControlIndex,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                if let Some(slot) = client.sizes.get_mut(index.0) {
                    *slot = Some(size);
                }
            }
            zwlr_gamma_control_v1::Event::Failed => {
                if let Some(slot) = client.failed.get_mut(index.0) {
                    *slot = true;
                }
            }
            _ => {}
        }
    }
}

/// Runs the client half: executes whatever steps the test sends, acknowledging
/// each one once the compositor has seen it.
fn run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    let registry = conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    while let Ok(step) = steps.recv() {
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        let outcome = match step {
            Step::Bind => {
                let (name, version) = client.output_name.ok_or("no wl_output")?;
                let _: wl_output::WlOutput = registry.bind(name, version.min(4), &qh, ());
                Ack::Done
            }
            Step::GetGamma => {
                let (name, version) = client
                    .manager_name
                    .ok_or("no zwlr_gamma_control_manager_v1")?;
                let manager: zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1 =
                    registry.bind(name, version.min(1), &qh, ());
                let (output_name, output_version) = client.output_name.ok_or("no wl_output")?;
                let output: wl_output::WlOutput =
                    registry.bind(output_name, output_version.min(4), &qh, ());
                let index = client.controls.len();
                client
                    .controls
                    .push(manager.get_gamma_control(&output, &qh, ControlIndex(index)));
                client.sizes.push(None);
                client.failed.push(false);
                // The `gamma_size` arrives after the object, never with it --
                // a bare round trip is not enough and cannot be made enough.
                let size = wait_for(&mut queue, &mut client, "gamma_size", |client| {
                    client.sizes[index]
                })?;
                Ack::GammaSize(size)
            }
            Step::TakeModes => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Modes(std::mem::take(&mut client.modes))
            }
            Step::TakeFailed => {
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                Ack::Failed(std::mem::take(&mut client.failed))
            }
        };
        queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
        acks.send(outcome).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A live compositor with a real headless backend and one connected client,
/// scripted a step at a time. See [`crate::compositor::test_support`] for
/// everything that is not specific to this outcome.
type Fixture = Harness<Step, Ack>;

impl Fixture {
    fn switched() -> Self {
        let mut fixture = Harness::headless(Appearance::default(), CANVAS);
        fixture.spawn(run_client);
        fixture
    }

    fn gamma_size(&mut self) -> u32 {
        match self.run(Step::GetGamma) {
            Ack::GammaSize(size) => size,
            _ => panic!("the client answered a gamma request with nothing"),
        }
    }

    fn modes(&mut self) -> Vec<SeenMode> {
        match self.run(Step::TakeModes) {
            Ack::Modes(modes) => modes,
            _ => panic!("the client answered a modes request with nothing"),
        }
    }

    fn failed(&mut self) -> Vec<bool> {
        match self.run(Step::TakeFailed) {
            Ack::Failed(failed) => failed,
            _ => panic!("the client answered a failed request with nothing"),
        }
    }
}

fn switched_crtc(width: i32, height: i32, size_changed: bool, gamma_size: u32) -> Reconfigured {
    Reconfigured::SwitchedCrtc {
        width,
        height,
        size_changed,
        gamma_size,
    }
}

#[test]
fn switched_crtc_with_new_size_resizes_output_and_updates_gamma() {
    let mut fixture = Fixture::switched();
    fixture.run(Step::Bind);
    // Sanity: the client sees the fixture's own output before anything moves.
    assert!(
        fixture.modes().contains(&SeenMode {
            width: CANVAS,
            height: CANVAS,
            current: true,
            preferred: true,
        }),
        "the client should see the fixture's own mode first"
    );
    assert_eq!(fixture.gamma_size(), FALLBACK_GAMMA_SIZE);

    switched_crtc(BIG, BIG, true, NEW_GAMMA_SIZE)
        .finish(&mut fixture.state, scoot_core::OutputId(1));

    // The size half is the `Resized` contract: `wl_output` agrees, current
    // *and* preferred (see `set_mode`'s ordering guarantee).
    assert!(
        fixture.modes().contains(&SeenMode {
            width: BIG,
            height: BIG,
            current: true,
            preferred: true,
        }),
        "a size-changing crtc switch must resize wl_output like Resized does"
    );
    // ...and the gamma half: a later control learns the new CRTC's length.
    assert_eq!(fixture.gamma_size(), NEW_GAMMA_SIZE);
}

#[test]
fn switched_crtc_at_same_size_requests_render_without_resize() {
    let mut fixture = Fixture::switched();
    fixture.run(Step::Bind);
    fixture.modes();
    fixture.state.needs_render = false;

    switched_crtc(CANVAS, CANVAS, false, NEW_GAMMA_SIZE)
        .finish(&mut fixture.state, scoot_core::OutputId(1));

    // A render is owed (the scanout was invalidated) but the output never
    // moved: every mode the client heard names the size it started at.
    assert!(
        fixture.state.needs_render,
        "a same-size crtc switch must still ask for a render"
    );
    assert!(
        fixture
            .modes()
            .iter()
            .all(|mode| mode.width == CANVAS && mode.height == CANVAS),
        "a same-size crtc switch must not resize the output"
    );
    assert_eq!(fixture.gamma_size(), NEW_GAMMA_SIZE);
}

#[test]
fn switched_crtc_with_new_gamma_fails_live_control() {
    let mut fixture = Fixture::switched();
    fixture.run(Step::Bind);
    assert_eq!(fixture.gamma_size(), FALLBACK_GAMMA_SIZE);

    switched_crtc(CANVAS, CANVAS, false, NEW_GAMMA_SIZE)
        .finish(&mut fixture.state, scoot_core::OutputId(1));

    // The live control was sized for the old CRTC: it hears `failed` (the
    // transfer shape) rather than eating `invalid_gamma` on its next set_gamma.
    assert_eq!(fixture.failed(), vec![true]);
    // ...and a control made afterwards learns the new length straight away.
    assert_eq!(fixture.gamma_size(), NEW_GAMMA_SIZE);
}

#[test]
fn switched_crtc_with_same_gamma_fails_live_control_so_ramp_is_repushed() {
    let mut fixture = Fixture::switched();
    fixture.run(Step::Bind);
    assert_eq!(fixture.gamma_size(), FALLBACK_GAMMA_SIZE);

    switched_crtc(CANVAS, CANVAS, false, FALLBACK_GAMMA_SIZE)
        .finish(&mut fixture.state, scoot_core::OutputId(1));

    // Same length but a different CRTC: nothing pushes the old ramp to the
    // new hardware (a modeset carries plane state, not LUT contents), so
    // the live control hears `failed` and re-pushes rather than showing
    // un-warmed white until its next periodic set.
    assert_eq!(fixture.failed(), vec![true]);
}
