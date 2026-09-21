//! Session lock across more than one output (milestone 19, phase C).
//!
//! The per-output pins: each admitted surface is sized to its *own* output,
//! the locked render path draws each output's surface onto that output alone,
//! keyboard focus follows the pointer's output among the lock surfaces, and
//! `locked` waits for *every* output's blanked frame -- confirming before
//! every screen has blanked would expose a live desktop on the unblanked
//! screen, which is the bug the vblank-confirmation work exists to prevent.
//! The duplicate-output refusal (one live surface per physical output) stays
//! as-is and is pinned still firing.
//!
//! Like the other suites here these drive a real `wayland-client`
//! connection and read pixels per output, so "the right surface's wrong
//! screen" cannot pass. The canvas is square and both outputs are the same
//! size unless a test says otherwise, placed side by side: output 1 covers
//! `0..CANVAS` on both axes, output 2 covers `CANVAS..2*CANVAS` on `x` and
//! `0..CANVAS` on `y`. Two colours -- one per output -- so a surface drawn
//! on the wrong screen reads unambiguously.

use super::*;
use crate::compositor::render::Backend;
use scoot_core::OutputId;

/// The client's `wl_output` registry index of the second output -- 0 is the
/// primary, matching the order `headless::add_output` creates them in.
const SECOND: usize = 1;
/// The global `x` the second output starts at: immediately right of the first.
const X2: f64 = CANVAS as f64;
/// The second output's lock surface colour: unmistakable against every colour
/// `mod.rs` already uses, so cross-output leaks read unambiguously.
const LOCK2_BGRA: [u8; 4] = [0xE0, 0xE0, 0x20, 0xFF];

/// A live compositor with two side-by-side outputs and one connected client.
///
/// The extra output is added before the client connects, so the registry
/// announces the primary's global first and the second output's second --
/// which is what makes `outputs[0]` the primary and `outputs[1]` the second.
fn two_output_fixture() -> Fixture {
    let mut fixture = Harness::headless(appearance(), CANVAS);
    crate::compositor::headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second headless output");
    fixture.connect();
    fixture
}

/// The configured sizes in creation order, dropping the serials (which are
/// session-global, not per-surface).
fn configured_sizes(report: &Report) -> Vec<Option<(u32, u32)>> {
    report
        .lock_configures
        .iter()
        .map(|configure| configure.map(|(_, width, height)| (width, height)))
        .collect()
}

/// Takes every output's render target, so no frame tick can blank or confirm
/// early while a lock is set up -- the two-output version of
/// `take_primary_backend` (see `blanking.rs` for why the single-output suite
/// needs it too: admitting a surface arms the frame timer, and `settle`
/// gives it time to fire).
fn take_all_backends(fixture: &mut Fixture) -> Vec<(OutputId, Backend)> {
    let mut taken = Vec::new();
    for id in [OutputId(1), OutputId(2)] {
        taken.push((id, fixture.state.take_backend(id).expect("a backend")));
    }
    taken
}

/// Puts every taken render target back, in the order it was taken.
fn put_all_backends(fixture: &mut Fixture, taken: Vec<(OutputId, Backend)>) {
    for (id, backend) in taken {
        fixture.state.put_backend(id, backend);
    }
}

/// Draws every output and hands back both outputs' raw BGRA pixels.
fn render_all(fixture: &mut Fixture) -> (Vec<u8>, Vec<u8>) {
    fixture.state.request_render();
    fixture.state.render();
    (
        fixture.pixels_of(OutputId(1)),
        fixture.pixels_of(OutputId(2)),
    )
}

/// Draws every output and asserts each one is whole-screen its own colour.
fn assert_outputs_are(first: &[u8], second: &[u8], what: &str) {
    for y in 0..CANVAS {
        for x in 0..CANVAS {
            let found = pixel(first, x, y);
            assert_eq!(
                found, LOCK_BGRA,
                "{what}: output 1 pixel ({x}, {y}) is {found:?}, expected {LOCK_BGRA:?}"
            );
            let found = test_support::pixel(second, CANVAS, x, y);
            assert_eq!(
                found, LOCK2_BGRA,
                "{what}: output 2 pixel ({x}, {y}) is {found:?}, expected {LOCK2_BGRA:?}"
            );
        }
    }
}

/// Surfaces on both outputs are each configured to their own output's size,
/// each draws whole-screen on its own output, and `locked` arrives.
/// Fail-first twice over: drawing every surface onto every output (the old
/// locked render path) puts the first surface's colour on output 2, and
/// confirming on the first blanked frame is unobservable here only because
/// both surfaces are up before the first frame -- the ordering half is the
/// next test's.
#[test]
fn surfaces_on_both_outputs_each_cover_their_own_output() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND,
        color: Some(LOCK2_BGRA),
    });

    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "both surfaces drawn, so `locked` must have arrived"
    );
    assert_eq!(
        configured_sizes(&report),
        vec![
            Some((CANVAS as u32, CANVAS as u32)),
            Some((CANVAS as u32, CANVAS as u32)),
        ],
        "each surface is configured to its own output's size"
    );

    let (first, second) = render_all(&mut fixture);
    assert_outputs_are(&first, &second, "each output shows its own lock surface");
}

/// The keyboard follows the pointer's output among the lock surfaces: one
/// surface per output, and exactly one holds the keyboard at a time.
///
/// Derivation happens where it always has -- when a surface maps or unmaps,
/// and at lock transitions -- and the pointer's output picks the surface,
/// the milestone-19 focus decision applied to the existing derivation.
/// Fail-first: deriving the keyboard from the first current surface keeps it
/// on output 1 wherever the pointer is.
#[test]
fn keyboard_follows_the_pointer_output_among_lock_surfaces() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    // The pointer starts on the primary, so the first surface takes the
    // keyboard when it maps.
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });
    assert_eq!(
        fixture.report().keyboard_focus,
        Some(Which::Lock(0)),
        "the control: the first surface holds the keyboard while the pointer is on output 1"
    );

    // Move the pointer onto output 2, then map its surface: the derivation
    // that mapping provokes follows the pointer's output.
    fixture.state.pointer_move(X2 + 60.0, 60.0);
    fixture.settle();
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND,
        color: Some(LOCK2_BGRA),
    });
    assert_eq!(
        fixture.report().keyboard_focus,
        Some(Which::Lock(1)),
        "mapping a lock surface where the pointer is takes the keyboard there"
    );

    // Unmapping it hands the keyboard back to the remaining one -- still
    // exactly one focused surface, never zero and never both. The pointer is
    // still on output 2, which now has no surface, so the fallback (the
    // first current surface) answers.
    fixture.run(Step::DestroyLockSurface { index: 1 });
    assert_eq!(
        fixture.report().keyboard_focus,
        Some(Which::Lock(0)),
        "the keyboard falls back to the surviving surface when the second goes away"
    );
}

/// A lock covering only output 1: output 2 blanks with no surface -- the
/// solid-colour fallback -- and `locked` still arrives, because a backdrop
/// frame *is* that output's locked frame. A regression pin rather than
/// fail-first: pre-fix output 2 reads black too, but only because the one
/// surface is drawn at an off-target origin and clipped away -- this pins
/// that it blanks *by backdrop* while `locked` still arrives.
#[test]
fn an_output_with_no_surface_blanks_with_no_surface() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });

    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "output 2's backdrop frame is its locked frame, so `locked` must arrive"
    );

    let (first, second) = render_all(&mut fixture);
    assert_whole_screen_is(&first, LOCK_BGRA, "output 1 shows its lock surface");
    assert_whole_screen_is(&second, BLACK_BGRA, "output 2 blanks with no surface");
}

/// The load-bearing ordering: `locked` fires only after the LAST output's
/// blank, not the first. A surface admitted for output 2 but not yet drawn
/// holds the confirmation open -- output 2's screen is showing a placeholder,
/// not the locker's blank -- and mapping it releases it.
/// Fail-first: confirming on the first blanked frame sends `locked` after the
/// first render, so the middle assertion fails pre-fix.
#[test]
fn locked_waits_for_every_output_blank() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::MapWindow);
    // No render target: the accept runs, but no frame tick can blank or
    // confirm early -- without this the zero-surface frames `settle` lets
    // through would confirm the lock before either surface exists.
    let taken = take_all_backends(&mut fixture);
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    let ack = fixture.wait_for_ack(0);
    assert!(matches!(ack, Ack::Done));
    assert!(
        fixture.state.session_lock.awaiting_blank(),
        "with no frame drawn, the blanked frame is still outstanding"
    );
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });
    // Admitted and acked, but no buffer: the locker is mid-startup on output
    // 2, whose screen shows the backdrop placeholder.
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND,
        color: None,
    });
    put_all_backends(&mut fixture, taken);

    fixture.state.request_render();
    fixture.state.render();
    assert_eq!(
        fixture.report().locked,
        0,
        "`locked` must not go out while output 2's surface hasn't drawn"
    );
    assert!(
        fixture.state.session_lock.awaiting_blank(),
        "the lock is still waiting for output 2's blank"
    );

    // The locker finishes starting up: the same-size buffer its configure
    // promised, committed at it.
    fixture.run(Step::ReattachLockBuffer { index: 1 });
    fixture.state.request_render();
    fixture.state.render();
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "`locked` fires once the last output's blank exists"
    );
    assert!(
        fixture.state.session_lock.pending.is_none(),
        "the wait is over"
    );
}

/// The duplicate-output refusal still fires per physical output with two of
/// them: covering both, then naming the second output again -- through the
/// client's second bind of its global -- kills the client with
/// `duplicate_output` and leaves the session locked. A regression pin rather
/// than fail-first: the refusal is the behaviour that stays as-is.
#[test]
fn duplicate_refusal_still_fires_per_physical_output() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND,
        color: Some(LOCK2_BGRA),
    });

    // `output2` is a second bind of the second output's global (see
    // `mod.rs`): the shape Smithay's resource-identity guard admits and
    // scoot refuses per physical output.
    let error = fixture.run_expecting_disconnect(Step::LockSurfaceSecondBind {
        lock: 0,
        color: Some(LOCK_BGRA),
    });
    assert!(
        error.contains("Protocol error 3 on object ext_session_lock_v1@")
            && error.contains("Output already has a lock surface"),
        "the client should die refused with duplicate_output: {error}"
    );
    assert!(
        fixture.state.session_lock.is_locked(),
        "a dead locker must not unlock the session"
    );
    fixture.render();
}

/// Each surface is sized to its own output when the outputs differ: 120x120
/// against 80x60. The white-box leg pins the configure sizes; the pixels leg
/// pins that each surface covers exactly its own output; the client's
/// survival pins that exact-size buffers were accepted on both. Fail-first on
/// the pixels: the old render path draws the 80-wide surface onto the
/// 120-wide output (and vice versa), so neither output is whole-screen its
/// own colour.
#[test]
fn each_surface_is_sized_to_its_own_output() {
    const W2: i32 = 80;
    const H2: i32 = 60;
    let mut fixture = Harness::headless(appearance(), CANVAS);
    crate::compositor::headless::add_output(&mut fixture.state, "headless-2", W2, H2)
        .expect("a smaller second headless output");
    fixture.connect();

    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND,
        color: Some(LOCK2_BGRA),
    });

    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "both surfaces drawn, so `locked` must have arrived"
    );
    assert_eq!(
        configured_sizes(&report),
        vec![
            Some((CANVAS as u32, CANVAS as u32)),
            Some((W2 as u32, H2 as u32)),
        ],
        "each surface is configured to its own output's size"
    );

    // `pixels_of` asserts the canvas size, which the second output is not --
    // read both framebuffers back directly instead.
    fixture.state.request_render();
    fixture.state.render();
    let first = read_output(&mut fixture, OutputId(1), CANVAS, CANVAS);
    let second = read_output(&mut fixture, OutputId(2), W2, H2);
    assert_whole_screen_is(&first, LOCK_BGRA, "output 1 shows its own lock surface");
    for y in 0..H2 {
        for x in 0..W2 {
            let found = test_support::pixel(&second, W2, x, y);
            assert_eq!(
                found, LOCK2_BGRA,
                "output 2 pixel ({x}, {y}) is {found:?}, expected {LOCK2_BGRA:?}"
            );
        }
    }
}

/// Reads output `id`'s framebuffer back without rendering first, for an
/// output whose size is not the harness canvas.
fn read_output(fixture: &mut Fixture, id: OutputId, width: i32, height: i32) -> Vec<u8> {
    let backend = fixture.state.backends.get_mut(&id).expect("a backend");
    assert_eq!(
        backend.size(),
        (width, height),
        "the output is the size the test built it at"
    );
    backend
        .capture(<[u8]>::to_vec)
        .expect("a framebuffer readback")
}

/// Unlocking restores both outputs: the window on output 1, the desktop
/// background on output 2, the keyboard back on the window. A regression pin
/// rather than fail-first: the unlocked path has been per-output since phase
/// A, so this passes pre-fix and pins that the lock changed nothing about
/// the way back.
#[test]
fn unlock_restores_both_outputs() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND,
        color: Some(LOCK2_BGRA),
    });
    let (locked_first, locked_second) = render_all(&mut fixture);
    assert!(
        !test_support::contains(&locked_first, WINDOW_BGRA)
            && !test_support::contains(&locked_second, WINDOW_BGRA),
        "the control: the window is on neither screen while locked"
    );

    fixture.run(Step::Unlock { lock: 0 });
    let (first, second) = render_all(&mut fixture);
    assert!(
        test_support::contains(&first, WINDOW_BGRA),
        "output 1 shows the window again after unlock"
    );
    assert_whole_screen_is(&second, BACKGROUND_BGRA, "output 2 shows the desktop again");
    assert_eq!(
        fixture.report().keyboard_focus,
        Some(Which::Window(0)),
        "the keyboard is back on the window"
    );
}

/// A lock surface destroyed mid-confirmation unblocks its output: with no
/// current surface there the backdrop is that output's locked frame, so the
/// next frame confirms instead of hanging. Fail-first on the middle
/// assertion: confirming on the first blanked frame sends `locked` after the
/// first render.
#[test]
fn destroying_a_surface_mid_confirmation_unblocks_its_output() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::MapWindow);
    let taken = take_all_backends(&mut fixture);
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    let ack = fixture.wait_for_ack(0);
    assert!(matches!(ack, Ack::Done));
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND,
        color: None,
    });
    put_all_backends(&mut fixture, taken);

    fixture.state.request_render();
    fixture.state.render();
    assert_eq!(
        fixture.report().locked,
        0,
        "the control: output 2's undrawn surface is still holding the confirmation"
    );

    fixture.run(Step::DestroyLockSurface { index: 1 });
    fixture.state.request_render();
    fixture.state.render();
    assert_eq!(
        fixture.report().locked,
        1,
        "with the surface gone, output 2's backdrop frame confirms the lock"
    );
}

/// The same, for the destruction that reaches no other hook: only the role
/// object goes, and the client then goes silent -- no commit that would tell
/// anyone. The orphan's screen shows the backdrop, finally, so it must count
/// like any other fallback rather than hang the locker. Fail-first against
/// the naive "unmapped blocks" rule with no torn-down carve-out.
#[test]
fn destroying_only_the_role_mid_confirmation_unblocks_its_output() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::MapWindow);
    let taken = take_all_backends(&mut fixture);
    fixture.send_step(0, Step::LockNoWait);
    fixture.settle();
    let ack = fixture.wait_for_ack(0);
    assert!(matches!(ack, Ack::Done));
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: 0,
        color: Some(LOCK_BGRA),
    });
    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND,
        color: None,
    });
    put_all_backends(&mut fixture, taken);

    fixture.state.request_render();
    fixture.state.render();
    assert_eq!(
        fixture.report().locked,
        0,
        "the control: output 2's undrawn surface is still holding the confirmation"
    );

    // Role gone, surface and connection kept, and then silence: no commit
    // follows, so anything that needs one to notice the teardown hangs.
    fixture.run(Step::DestroyLockRoleOnly { index: 1 });
    fixture.state.request_render();
    fixture.state.render();
    assert_eq!(
        fixture.report().locked,
        1,
        "a torn-down surface's backdrop is final, so it confirms like any fallback"
    );
}

/// A surface arriving after the lock already confirmed -- no surface anywhere
/// at lock time -- is sized to its output and shown, with no second
/// confirmation: `locked` was already sent and stays sent exactly once.
/// Fail-first on the pixels: the old render path draws the late surface onto
/// output 1 as well.
#[test]
fn a_surface_arriving_after_confirm_needs_no_second_confirmation() {
    let mut fixture = two_output_fixture();
    fixture.run(Step::MapWindow);
    fixture.run(Step::Lock);
    assert_eq!(
        fixture.report().locked,
        1,
        "the control: a zero-surface lock confirms on its blanked frames"
    );

    fixture.run(Step::LockSurfaceOn {
        lock: 0,
        output: SECOND,
        color: Some(LOCK2_BGRA),
    });
    let report = fixture.report();
    assert_eq!(
        report.locked, 1,
        "no second `locked`: the wait already completed"
    );
    assert_eq!(
        configured_sizes(&report),
        vec![Some((CANVAS as u32, CANVAS as u32))],
        "the late surface is sized to the output it named"
    );
    assert_eq!(
        report.keyboard_focus,
        Some(Which::Lock(0)),
        "the only surface holds the keyboard: the pointer's output has none"
    );
    assert_eq!(
        report.pointer_focus, None,
        "the pointer is over output 1, where there is no lock surface"
    );

    let (first, second) = render_all(&mut fixture);
    assert_whole_screen_is(
        &first,
        BLACK_BGRA,
        "output 1 keeps blanking with no surface",
    );
    assert_whole_screen_is(&second, LOCK2_BGRA, "output 2 shows the late surface");
}
