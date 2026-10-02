// The volume module's protocol parser, compiled from scootbar's source
// unchanged: it reaches nothing outside `proto` (which uses nothing but
// `std`). `fuzz` holds what the target checks, shared with scootbar's
// stable test that replays the corpus and `regressions/`
// (`modules/volume/fuzz.rs`).

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/modules/volume/proto.rs"]
pub mod proto;

#[rustfmt::skip]
#[path = "../../src/modules/volume/fuzz.rs"]
pub mod fuzz;
