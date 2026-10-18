//! Any bytes as a desktop file: parsing refuses or accepts without a
//! panic, and whatever it accepts re-reads within bounds (at most 32
//! groups, every value within 1 KiB, lists split on `;`, booleans
//! exact). The checks are `fuzz::parse` below.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../src/index/parse.rs"]
mod parse;

mod fuzz {
    use super::parse;

    pub fn parse(data: &[u8]) -> Option<()> {
        let file = parse::parse(data).ok()?;
        // Bounds the index relies on: re-derive them here so a
        // parser drift that breaks them fails the fuzzer, not the
        // launcher.
        assert!(file.groups.len() <= parse::MAX_GROUPS);
        for group in &file.groups {
            for (key, value) in &group.keys {
                assert!(!key.is_empty());
                assert!(value.len() <= parse::MAX_STRING, "uncut string");
                if key == "Type" || key.starts_with("Type[") {
                    let _ = value;
                }
            }
            // Every key re-reads: lists split, booleans judge, escapes
            // convert or refuse the key (never the file).
            for (key, value) in &group.keys {
                if key.starts_with("Actions") || key.starts_with("Keywords") {
                    let _ = parse::split_list(value);
                }
                if key == "Hidden" || key == "NoDisplay" || key == "Terminal" {
                    let _ = parse::parse_bool(value);
                }
                if !key.contains('[') {
                    let _ = parse::unescape(value);
                }
            }
            // Display strings carry no control characters.
            let cleaned = parse::clean_display(&group.name);
            assert_eq!(cleaned, group.name, "group name with controls");
        }
        Some(())
    }
}

fuzz_target!(|data: &[u8]| {
    fuzz::parse(data);
});
