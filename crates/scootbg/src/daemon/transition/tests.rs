//! Model tests for the daemon's transition driver that need no
//! compositor: the pure step decisions and row addressing. Frame blending
//! is `crate::transition`'s, covered there; the running-against-a-
//! compositor half is `crates/scootbg/tests/transition.rs`.

use super::{Endpoint, endpoint_row, step_due};
use crate::color::Color;

#[test]
fn steps_fire_once_each() {
    // A 32 ms transition is two frames: step 0, then 1, then done.
    assert!(!step_due(0, 32, 0));
    assert!(step_due(0, 32, 16));
    assert!(!step_due(1, 32, 16));
    assert!(step_due(1, 32, 32));
    assert!(!step_due(2, 32, 32));
    // Past the end the step saturates: nothing more is due (finishing is
    // the eased progress reaching 1, not the step moving).
    assert!(!step_due(2, 32, 10_000));
}

#[test]
fn endpoint_rows_address_every_row() {
    // A solid endpoint is one scanline reused for every row; shared
    // pixels address by `y`. (The first version indexed the scanline by
    // `y` and panicked past the first row.)
    let red = Color::parse("#c03020").unwrap().xrgb8888();
    let solid = [red[0], red[1], red[2], red[3]];
    let endpoint = Endpoint::Solid(Color::parse("#c03020").unwrap());
    for y in 0..4u32 {
        assert_eq!(endpoint_row(&endpoint, &solid, y, 4), &red[..], "row {y}");
    }
}
