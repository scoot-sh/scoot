// The tray's, the media module's and the bluetooth module's D-Bus wire
// parsers, compiled from scootbar's source unchanged: they reach nothing
// outside `proto` (which uses nothing but `std`). `fuzz` holds what the
// target checks, shared with scootbar's stable test that replays the
// corpus and `regressions/` (`src/dbus/fuzz.rs`).

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/dbus/proto.rs"]
pub mod proto;

// The media module's MPRIS shapes: `proto` and nothing else, as well.
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/dbus/mpris.rs"]
pub mod mpris;

// The bluetooth module's BlueZ shapes: `proto` and nothing else, as well.
#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/dbus/bluez.rs"]
pub mod bluez;

#[rustfmt::skip]
#[path = "../../src/dbus/fuzz.rs"]
pub mod fuzz;
