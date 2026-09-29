//! Any bytes as a `--clock-format`: parsing refuses or accepts without a
//! panic, and an accepted format renders any instant in any zone offset
//! without a panic, without a control character, and within a bound. The
//! checks are `fuzz::format` (scootbar's `modules/clock/fuzz.rs`).

#![no_main]

include!("common.rs");

libfuzzer_sys::fuzz_target!(|data: &[u8]| fuzz::format(data));
