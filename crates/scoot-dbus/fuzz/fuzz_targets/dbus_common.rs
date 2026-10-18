// The tray's, the media module's and the bluetooth module's D-Bus wire
// parsers, compiled from scoot-dbus's source unchanged: they reach nothing
// outside `proto` (which uses nothing but `std`). `fuzz` holds what the
// target checks, shared with scoot-dbus's stable test that replays the
// corpus and `regressions/` (`src/fuzz.rs`).

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/proto.rs"]
pub mod proto;

// The media module's MPRIS shapes: `proto` and nothing else, as well.
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/mpris.rs"]
pub mod mpris;

// The bluetooth module's BlueZ shapes: `proto` and nothing else, as well.
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/bluez.rs"]
pub mod bluez;

#[rustfmt::skip]
#[path = "../../src/fuzz.rs"]
pub mod fuzz;
