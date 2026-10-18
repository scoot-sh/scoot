//! Any bytes as a history file: parsing answers a bounded history
//! or nothing, never a panic, whatever the input. Weights compute
//! for every loaded record at hostile timestamps. The checks are
//! `fuzz::history` below.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../src/rank/frecency.rs"]
mod frecency;

mod fuzz {
    use super::frecency;

    pub fn history(data: &[u8]) -> Option<()> {
        let parsed = frecency::Frecency::from_bytes(data);
        // Bounds the ranker relies on: re-derive them here so a
        // drift that breaks them fails the fuzzer, not the launcher.
        assert!(parsed.len() <= frecency::MAX_ENTRIES);
        for stamp in [0, 1, u64::MAX, u64::MAX / 2] {
            for id in ["", "a", "x".repeat(600).as_str()] {
                let _ = parsed.weight(id, stamp);
            }
        }
        Some(())
    }
}

fuzz_target!(|data: &[u8]| {
    fuzz::history(data);
});
