//! Any bytes as netlink datagrams: framing accepts or stops without a
//! panic, and every message and attribute parser accepts or refuses
//! without one. The check is `fuzz::network` (scootbar's
//! `modules/network/fuzz.rs`).

#![no_main]

include!("network_common.rs");

libfuzzer_sys::fuzz_target!(|data: &[u8]| fuzz::network(data));
