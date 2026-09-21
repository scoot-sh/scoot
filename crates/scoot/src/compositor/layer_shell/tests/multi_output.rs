//! Layer shell across more than one output (milestone 19, phase B).
//!
//! The per-output pins: a bar's exclusive zone shrinks the usable area of
//! the output it is mapped on and no other's, pointer hit-testing and
//! keyboard focus follow the output under the pointer, and a surface that
//! names no output lands on the primary. The two harms in scope -- a zone
//! misapplied across outputs, and keyboard focus delivered to a surface on
//! the wrong output -- are both pinned fail-first below.
//!
//! Like the other suites here these drive a real `wayland-client`
//! connection, and read zones off `world.usable_area` per output id plus the
//! framebuffers per output, so "the right ids' wrong pixels" cannot pass.
//! The canvas is square and both outputs are the same size, placed side by
//! side: output 1 covers `0..CANVAS` on both axes, output 2 covers
//! `CANVAS..2*CANVAS` on `x` and `0..CANVAS` on `y`.

use super::*;
use scoot_core::OutputId;
use smithay::utils::Point;

/// The client's `wl_output` registry index of the second output -- 0 is the
/// primary, matching the order `headless::add_output` creates them in.
const SECOND: usize = 1;
/// The global `x` the second output starts at: immediately right of the first.
const X2: f64 = CANVAS as f64;

/// A live compositor with two side-by-side outputs and one connected client.
fn two_output_fixture() -> Fixture {
    let mut fixture = Harness::headless(appearance(), CANVAS);
    crate::compositor::headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second headless output");
    fixture.spawn(run_client);
    fixture
}

/// The usable area the core currently arranges output `id`'s windows within.
fn usable(fixture: &Fixture, id: u64) -> Rect {
    fixture
        .state
        .world
        .usable_area(OutputId(id))
        .expect("both outputs are known to the core")
}

/// Both outputs' whole areas, for the "untouched" halves of the assertions.
const WHOLE_FIRST: Rect = Rect::new(0, 0, CANVAS, CANVAS);
const WHOLE_SECOND: Rect = Rect::new(CANVAS, 0, CANVAS, CANVAS);

/// Draws every output and hands back output `id`'s raw BGRA pixels.
fn draw(fixture: &mut Fixture, id: u64) -> Vec<u8> {
    fixture.state.request_render();
    fixture.state.render();
    fixture.pixels_of(OutputId(id))
}

/// Creates a bar of `height` (reserving exactly its own height) on `output`
/// -- `None` for no requested output, the protocol's "the compositor
/// chooses" -- and maps it with a buffer. `index` is the surface's creation
/// index, counted by the test: these scripts map surfaces in a fixed order,
/// so the first is 0, the second 1, and so on.
fn map_bar(fixture: &mut Fixture, index: usize, output: Option<usize>, height: u32) {
    fixture.run(match output {
        Some(output) => Step::CreateLayerOn {
            spec: LayerSpec::bar(height),
            output,
        },
        None => Step::CreateLayer(LayerSpec::bar(height)),
    });
    fixture.run(Step::MapLayer {
        index,
        color: BAR_BGRA,
    });
}

// ---------------------------------------------------------------------------
// Exclusive zones: one output's bar shrinks one output's usable area
// ---------------------------------------------------------------------------

/// A bar on the second output shrinks the second output's usable area and
/// leaves the first output's alone -- the zone-leak harm, fail-first: before
/// per-output zones nothing on output 2 reserved anything anywhere.
#[test]
fn a_bar_on_the_second_output_shrinks_only_the_second_output() {
    let mut fixture = two_output_fixture();
    map_bar(&mut fixture, 0, Some(SECOND), 24);

    assert_eq!(
        usable(&fixture, 1),
        WHOLE_FIRST,
        "output 1's usable area is untouched by output 2's bar"
    );
    assert_eq!(
        usable(&fixture, 2),
        Rect::new(CANVAS, 24, CANVAS, CANVAS - 24),
        "output 2's usable area shrinks by its own bar's height"
    );

    // ...and the pixels agree: the bar draws on output 2, not output 1.
    let first = draw(&mut fixture, 1);
    let second = draw(&mut fixture, 2);
    assert!(
        !contains(&first, BAR_BGRA),
        "output 1 shows its own strip, not output 2's bar"
    );
    assert_pixel(
        &second,
        CANVAS / 2,
        0,
        BAR_BGRA,
        "the bar covers the second output's top row",
    );
}

/// Bars on both outputs each hold their own zone: neither shrinks the other.
#[test]
fn bars_on_both_outputs_hold_both_zones() {
    let mut fixture = two_output_fixture();
    map_bar(&mut fixture, 0, Some(0), 30);
    map_bar(&mut fixture, 1, Some(SECOND), 24);

    assert_eq!(
        usable(&fixture, 1),
        Rect::new(0, 30, CANVAS, CANVAS - 30),
        "output 1's usable area shrinks by its own bar's height"
    );
    assert_eq!(
        usable(&fixture, 2),
        Rect::new(CANVAS, 24, CANVAS, CANVAS - 24),
        "output 2's usable area shrinks by its own bar's height"
    );
}

/// A surface that names no output lands on the primary: the protocol's "the
/// compositor chooses", and scoot's choice. Pins the default rather than the
/// mechanism -- what matters is that an unrequested bar reserves the first
/// output's edge and nothing on the second.
#[test]
fn a_layer_surface_without_a_requested_output_lands_on_the_primary() {
    let mut fixture = two_output_fixture();
    map_bar(&mut fixture, 0, None, 24);

    assert_eq!(
        usable(&fixture, 1),
        Rect::new(0, 24, CANVAS, CANVAS - 24),
        "an unrequested bar reserves the primary output's edge"
    );
    assert_eq!(
        usable(&fixture, 2),
        WHOLE_SECOND,
        "an unrequested bar reserves nothing on the second output"
    );
}

/// Disconnecting a bar mid-session on output 2 releases its zone there and
/// leaves output 1's alone.
#[test]
fn disconnecting_a_second_output_bar_releases_its_zone() {
    let mut fixture = two_output_fixture();
    map_bar(&mut fixture, 0, Some(SECOND), 24);
    assert_eq!(
        usable(&fixture, 2),
        Rect::new(CANVAS, 24, CANVAS, CANVAS - 24),
        "the control: the zone really applied before the disconnect"
    );

    fixture.disconnect_client();

    assert_eq!(
        usable(&fixture, 2),
        WHOLE_SECOND,
        "output 2's usable area is restored once its bar is gone"
    );
    assert_eq!(
        usable(&fixture, 1),
        WHOLE_FIRST,
        "output 1's usable area is unaffected throughout"
    );
}

// ---------------------------------------------------------------------------
// Input and keyboard focus follow the output under the pointer
// ---------------------------------------------------------------------------

/// Clicking a bar on the second output focuses it -- pointer and keyboard
/// both follow the pointer's output. Fail-first: hit-testing the primary's
/// map only misses a position over another output, so the click lands on the
/// desktop and the keyboard goes nowhere.
#[test]
fn clicking_a_bar_on_the_second_output_focuses_it() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::CreateLayerOn {
        spec: LayerSpec::bar(24).with_keyboard(KeyboardInteractivity::OnDemand),
        output: SECOND,
    });
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    // Over the bar in global coordinates: output 2 starts at `X2`.
    fixture.click(X2 + 100.0, 12.0);

    assert_eq!(
        fixture.pointer_focus(),
        Some(Focused::Layer(0)),
        "the pointer entered the second output's bar, not the desktop behind it"
    );
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the click gave the second output's bar the keyboard"
    );
}

/// One `exclusive` surface per output: the pointer's output wins, and exactly
/// one surface holds the keyboard at a time. Fail-first on the second half:
/// deriving focus from the primary's map only keeps the first launcher
/// focused wherever the pointer is.
#[test]
fn exclusive_surfaces_on_both_outputs_the_pointer_output_wins() {
    let mut fixture = two_output_fixture();
    // The pointer starts at the primary's centre, so the first launcher
    // takes the keyboard when it maps.
    fixture.run(Step::CreateLayerOn {
        spec: LayerSpec::launcher(60),
        output: 0,
    });
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the control: the first launcher holds the keyboard while the pointer is on output 1"
    );

    // Move the pointer onto output 2, then map its launcher: the derivation
    // that mapping provokes follows the pointer's output.
    fixture.state.pointer_move(X2 + 170.0, 170.0);
    fixture.settle();
    fixture.run(Step::CreateLayerOn {
        spec: LayerSpec::launcher(60),
        output: SECOND,
    });
    fixture.run(Step::MapLayer {
        index: 1,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(1)),
        "mapping a launcher where the pointer is takes the keyboard there"
    );

    // Unmapping it hands the keyboard back to the remaining one -- still
    // exactly one focused surface, never zero and never both.
    fixture.run(Step::DestroyLayer { index: 1 });
    assert_eq!(
        fixture.keyboard().focused,
        Some(Focused::Layer(0)),
        "the keyboard falls back to the surviving launcher when the second goes away"
    );
}

/// A fast relative fling across the seam lands on the second output and
/// enters its bar there (milestone 19, phase D): the union clamp lets
/// libinput-rate motion cross outputs -- hundreds of per-event clamps in a
/// row with no focus or event loss -- and the per-output hit test applies
/// unchanged on arrival. Fail-first: clamping to the primary holds every
/// step at 199, so the pointer never leaves output 1 and the bar is never
/// entered.
#[test]
fn a_relative_fling_across_the_seam_enters_the_second_outputs_bar() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::CreateLayerOn {
        spec: LayerSpec::bar(24).with_keyboard(KeyboardInteractivity::OnDemand),
        output: SECOND,
    });
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });

    // From deep on output 1, hundreds of single-pixel relative steps -- the
    // rate and shape of a real fling -- across the seam at `X2` and up onto
    // the bar, which spans `X2..2*X2` on `x` and the top 24 rows.
    fixture.state.pointer_move(50.0, 100.0);
    for _ in 0..250 {
        fixture.state.pointer_move_relative(1.0, 0.0, 1.0, 0.0);
    }
    for _ in 0..88 {
        fixture.state.pointer_move_relative(0.0, -1.0, 0.0, -1.0);
    }
    let location = fixture
        .state
        .seat
        .get_pointer()
        .expect("a pointer")
        .current_location();
    assert_eq!(
        (location.x, location.y),
        (300.0, 12.0),
        "the fling must cross the seam instead of sticking at output 1's edge"
    );
    let (output, _) = fixture
        .state
        .output_under(Point::from((location.x, location.y)))
        .expect("the landing point is on an output");
    assert_eq!(
        fixture.state.outputs.id_of(&output),
        Some(OutputId(2)),
        "the landing point resolves to the second output"
    );
    assert_eq!(
        fixture.pointer_focus(),
        Some(Focused::Layer(0)),
        "the flung pointer entered the second output's bar, with no click and no focus loss"
    );
}
