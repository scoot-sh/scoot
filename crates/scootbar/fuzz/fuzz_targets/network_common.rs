// The network module's netlink parser, compiled from scootbar's source
// unchanged: it reaches nothing outside `netlink` (which uses nothing but
// `std`). `fuzz` holds what the target checks, shared with scootbar's
// stable test that replays the corpus and `regressions/`
// (`modules/network/fuzz.rs`).

#[allow(dead_code)]
#[rustfmt::skip]
#[path = "../../src/modules/network/netlink.rs"]
pub mod netlink;

#[rustfmt::skip]
#[path = "../../src/modules/network/fuzz.rs"]
pub mod fuzz;
