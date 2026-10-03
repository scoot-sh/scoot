// The tray's D-Bus wire parser, compiled from scootbar's source
// unchanged: it reaches nothing outside `proto` (which uses nothing but
// `std`). `fuzz` holds what the target checks, shared with scootbar's
// stable test that replays the corpus and `regressions/`
// (`src/dbus/fuzz.rs`).

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/dbus/proto.rs"]
pub mod proto;

#[rustfmt::skip]
#[path = "../../src/dbus/fuzz.rs"]
pub mod fuzz;
