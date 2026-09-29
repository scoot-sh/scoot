// The clock's parsers, compiled from scootbar's source unchanged: `format`
// reaches `tzif` as `super::tzif`, which resolves here as it does there.
// `fuzz` holds what each target checks, shared with scootbar's stable test
// that replays the corpus and `regressions/` (`modules/clock/fuzz.rs`).
// `rustfmt::skip` keeps this crate's `cargo fmt` from following them to the
// `cfg(test)` modules it cannot resolve from here.

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/modules/clock/tzif.rs"]
pub mod tzif;

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/modules/clock/format.rs"]
pub mod format;

#[rustfmt::skip]
#[path = "../../src/modules/clock/fuzz.rs"]
pub mod fuzz;
