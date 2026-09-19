//! `mapped_layers` across teardown.
//!
//! That set (`state.rs`) is written when a layer surface maps and removed
//! when its role object is destroyed -- but the role destruction never runs
//! when the `wl_surface` dies before its role object (the implicit-disconnect
//! order), so `CompositorHandler::destroyed` sweeps the dead entries itself,
//! next to where it clears the cursor and lock surfaces. These tests pin
//! that: the explicit surface-dead-first shape, which must fail without the
//! sweep; that the sweep drops only the dead entry; and the
//! whole-disconnect shape, which must hold no dead entries either way.

use super::*;
use smithay::utils::IsAlive;

// -------------------------------------------------------------------------
// `mapped_layers` across teardown
// -------------------------------------------------------------------------

/// The explicit form of the unlucky order: the `wl_surface` is destroyed
/// while its layer role object (and the connection) stays alive, so
/// `layer_destroyed` never runs for it. Without the `retain(alive)` sweep in
/// `CompositorHandler::destroyed`, the dead entry stays in `mapped_layers`.
#[test]
fn a_layer_surface_whose_wl_surface_died_first_leaves_no_dead_mapped_layer() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateLayer(LayerSpec::bar(10)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.state.mapped_layers.len(),
        1,
        "the mapped bar must be tracked"
    );
    assert!(
        fixture
            .state
            .mapped_layers
            .iter()
            .all(|surface| surface.alive()),
        "a mapped surface must be alive before the teardown"
    );
    fixture.run(Step::DestroyLayerWlSurface { index: 0 });
    assert!(
        fixture
            .state
            .mapped_layers
            .iter()
            .all(|surface| surface.alive()),
        "a dead wl_surface must not stay in mapped_layers once destroyed() has run"
    );
    fixture.disconnect_client();
    assert!(
        fixture.state.mapped_layers.is_empty(),
        "disconnect must not retain dead mapped_layers entries"
    );
}

/// The whole-disconnect shape the ticket names: a client with a mapped layer
/// surface goes away without destroying anything, and whatever callback
/// order the backend teardown takes, no dead entry may be left behind.
#[test]
fn disconnecting_with_a_mapped_layer_surface_leaves_no_dead_mapped_layers() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateLayer(LayerSpec::bar(10)));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    assert_eq!(
        fixture.state.mapped_layers.len(),
        1,
        "the mapped bar must be tracked"
    );
    fixture.disconnect_client();
    assert!(
        fixture.state.mapped_layers.is_empty(),
        "disconnect must not retain dead mapped_layers entries"
    );
}

/// The sweep removes only the dead: with two mapped surfaces, destroying
/// one `wl_surface` must leave exactly the live entry behind.
#[test]
fn the_sweep_keeps_live_mapped_layers_while_dropping_the_dead_one() {
    let mut fixture = Fixture::new();
    fixture.run(Step::CreateLayer(LayerSpec::bar(10)));
    fixture.run(Step::CreateLayer(LayerSpec::wallpaper()));
    fixture.run(Step::MapLayer {
        index: 0,
        color: BAR_BGRA,
    });
    fixture.run(Step::MapLayer {
        index: 1,
        color: WALLPAPER_BGRA,
    });
    assert_eq!(
        fixture.state.mapped_layers.len(),
        2,
        "both mapped surfaces must be tracked"
    );
    fixture.run(Step::DestroyLayerWlSurface { index: 0 });
    assert_eq!(
        fixture.state.mapped_layers.len(),
        1,
        "the sweep must drop exactly the dead entry"
    );
    assert!(
        fixture
            .state
            .mapped_layers
            .iter()
            .all(|surface| surface.alive()),
        "the surviving entry must be the live surface"
    );
    fixture.disconnect_client();
    assert!(
        fixture.state.mapped_layers.is_empty(),
        "disconnect must not retain dead mapped_layers entries"
    );
}
