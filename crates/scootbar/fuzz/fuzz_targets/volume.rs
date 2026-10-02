//! Any bytes as PulseAudio-protocol frames: framing accepts, waits or
//! refuses without a panic, and every reply parser accepts or refuses
//! without one. The check is `fuzz::volume` (scootbar's
//! `modules/volume/fuzz.rs`).

#![no_main]

include!("volume_common.rs");

libfuzzer_sys::fuzz_target!(|data: &[u8]| fuzz::volume(data));
