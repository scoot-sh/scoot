//! Frame callbacks across layer-surface teardown.
//!
//! A callback the compositor has taken but not answered is a client waiting
//! forever, which is how a bar stops redrawing; a callback answered twice is
//! a protocol error. Both are invisible to a test that only looks at pixels.

use super::*;

// -------------------------------------------------------------------------
// Frame callbacks across layer-surface teardown
// -------------------------------------------------------------------------

/// The control the test below is read against: a frame callback requested
/// on a mapped layer surface is completed by the next frame. Without this,
/// "no `done` arrived" below could pass because `done` never works at all
/// rather than because the teardown path is clean.
#[test]
fn a_frame_callback_on_a_mapped_layer_surface_is_completed() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.render();
    assert_eq!(
        fixture.frames(),
        vec![1],
        "one frame should complete one requested callback"
    );
    fixture.disconnect_client();
}

/// Dismissing a layer surface with a frame callback in flight must not
/// complete that callback afterwards: the client has torn the surface down
/// (`zwlr_layer_surface_v1.destroy` + null attach + commit, the exact
/// sequence DankMaterialShell sends when an overlay is dismissed), and a
/// `done` arriving for an object it no longer knows kills its whole
/// connection (measured 4/4 with Quickshell, exit 255 -- see
/// `docs/backlog/resolved/dms-reprobe-done.md`, gap 1).
///
/// Read as a delta, not an absolute: whatever frames were already sent
/// before the teardown was dispatched are legitimate and counted in both
/// probes, so only a *new* `done` after the destroy fails this.
#[test]
fn dismissing_a_layer_surface_with_a_frame_callback_in_flight_sends_no_done() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.run(Step::DismissLayer { index: 0 });
    let before = fixture.frames();
    // Several frames after the teardown was dispatched: none of them may
    // complete the dead surface's callback.
    fixture.render();
    fixture.render();
    fixture.render();
    assert_eq!(
        fixture.frames(),
        before,
        "no frame after the teardown may complete the dead callback"
    );
    fixture.disconnect_client();
}

/// More than one callback outstanding when the surface is dismissed: none
/// of them may complete afterwards either.
#[test]
fn dismissing_with_several_frame_callbacks_in_flight_sends_none() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.run(Step::RequestLayerFrame { index: 0 });
    fixture.run(Step::DismissLayer { index: 0 });
    let before = fixture.frames();
    assert_eq!(before.len(), 3);
    fixture.render();
    fixture.render();
    fixture.render();
    assert_eq!(
        fixture.frames(),
        before,
        "no frame after the teardown may complete any dead callback"
    );
    fixture.disconnect_client();
}

/// Hiding an already-hidden surface (a null commit on one that is already
/// unmapped) is meaningless but legal -- wlroots treats it as a no-op -- so
/// it must not kill the client either.
#[test]
fn unmapping_twice_without_remap_does_not_kill_the_client() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    fixture.run(Step::CreateLayer(LayerSpec::launcher(60)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));

    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));
    // Already hidden: a second null commit must be a silent no-op.
    fixture.run(Step::UnmapLayer { index: 0 });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Window(0)));

    // ...and the surface still works afterwards: re-showing it maps again.
    fixture.run(Step::RemapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(fixture.keyboard().focused, Some(Focused::Layer(0)));
    fixture.disconnect_client();
}

/// Rapid map/destroy churn: every destruction arms the neutralize path, and
/// the compositor must keep serving through all of it.
#[test]
fn rapid_map_and_destroy_cycles_leave_the_compositor_serving() {
    let mut fixture = Fixture::new();
    fixture.run(Step::MapWindow);
    for _ in 0..5 {
        let Ack::Done = fixture.run(Step::CreateLayer(LayerSpec::bar(10))) else {
            panic!("every step should acknowledge");
        };
    }
    // Map and destroy each of them in turn (indices 0..5 in creation order).
    for index in 0..5 {
        fixture.run(Step::MapLayer {
            index,
            color: BAR_BGRA,
        });
        fixture.run(Step::DestroyLayer { index });
    }
    assert_eq!(fixture.usable(), WHOLE);
    let pixels = fixture.render();
    assert_pixel(
        &pixels,
        CANVAS / 2,
        CANVAS / 2,
        BACKGROUND_BGRA,
        "nothing left drawn",
    );
    fixture.disconnect_client();
}
