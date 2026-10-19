//! Any bytes as dmenu stdin: splitting answers bounded lines,
//! never a panic, whatever the input. The checks are `fuzz::dmenu`
//! below.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../src/dmenu/lines.rs"]
mod lines;

mod fuzz {
    use super::lines;

    pub fn dmenu(data: &[u8]) -> Option<()> {
        let split = lines::split_input(data);
        // Bounds the panel relies on: re-derive them here so a drift
        // that breaks them fails the fuzzer, not the launcher.
        assert!(split.len() <= lines::MAX_LINES);
        for line in &split {
            assert!(line.len() <= lines::MAX_LINE_BYTES);
        }
        Some(())
    }
}

fuzz_target!(|data: &[u8]| {
    fuzz::dmenu(data);
});
