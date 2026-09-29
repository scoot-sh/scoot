//! Any bytes as a `--clock-format`: parsing refuses or accepts without a
//! panic, and an accepted format renders any instant in any zone offset
//! without a panic, without a control character, and within a bound.

#![no_main]

include!("common.rs");

use format::{Civil, Format};
use tzif::{Abbr, Local};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&seed, text)) = data.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(text) else {
        return;
    };
    let Ok(format) = Format::parse(text) else {
        return;
    };
    let offsets = [0, 93_599, -89_999, i32::from(seed) * 631 - 80_000];
    let instants = [i64::MIN, -1, 0, 1_790_651_220, i64::MAX, i64::from(seed) << 40];
    for offset in offsets {
        let local = Local {
            offset,
            dst: seed & 1 == 1,
            abbr: Abbr::new(&data[..data.len().min(10)]),
        };
        for t in instants {
            let mut out = String::new();
            format.render(&Civil::at(t, local), &mut out);
            assert!(!out.chars().any(char::is_control), "{text:?} gave {out:?}");
            assert!(out.len() <= text.len() * 10 + 16, "{text:?} gave {out:?}");
        }
    }
});
