//! Any bytes as D-Bus messages on a stream: framing accepts, waits or
//! refuses without a panic, and every header and body parser accepts or
//! refuses without one. The check is `fuzz::dbus` (scootbar's
//! `src/dbus/fuzz.rs`).

#![no_main]

include!("dbus_common.rs");

libfuzzer_sys::fuzz_target!(|data: &[u8]| fuzz::dbus(data));
