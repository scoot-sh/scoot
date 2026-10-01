//! Any bytes as an update payload: an `exec` line in either format, and, where
//! they are JSON, a `msg set` value. Refused or accepted without a panic; an
//! accepted update is bounded, printable and trimmed; a refused one changes
//! nothing. The check is `fuzz::payload` (scootbar's
//! `modules/payload/fuzz.rs`).

#![no_main]

include!("payload_common.rs");

libfuzzer_sys::fuzz_target!(|data: &[u8]| fuzz::payload(data));
