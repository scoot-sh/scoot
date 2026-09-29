//! Any bytes as a zone file, and as a POSIX TZ string: refused or read
//! without a panic, and a zone read answers any instant, within a day and
//! two hours of UTC. The checks are `fuzz::tzif` (scootbar's
//! `modules/clock/fuzz.rs`).

#![no_main]

include!("common.rs");

libfuzzer_sys::fuzz_target!(|data: &[u8]| fuzz::tzif(data));
