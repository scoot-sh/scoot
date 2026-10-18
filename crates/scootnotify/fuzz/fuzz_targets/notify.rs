#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = scootnotify::bus::notify::fuzz_check(data);
});
