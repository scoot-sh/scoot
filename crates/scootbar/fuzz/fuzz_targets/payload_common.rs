// The payload parser, compiled from scootbar's source unchanged: it reaches
// `Class` and `MAX_TEXT` as `super::{Class, MAX_TEXT}`, which this root
// re-exports as `modules` does. `fuzz` holds what the target checks, shared
// with scootbar's stable test that replays the corpus and `regressions/`
// (`modules/payload/fuzz.rs`).

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/modules/class.rs"]
pub mod class;

pub use class::{Class, MAX_TEXT};

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/modules/payload.rs"]
pub mod payload;

pub use payload::{Format, Shown, from_value, parse_line};

#[rustfmt::skip]
#[path = "../../src/modules/payload/fuzz.rs"]
pub mod fuzz;
