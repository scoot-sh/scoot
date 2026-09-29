//! What the fuzz targets check, one function per target, written once:
//! `crates/scootbar/fuzz` compiles this file by `#[path]` (so it uses
//! nothing but [`super::format`] and [`super::tzif`], as they use nothing
//! but `std`), and the stable test beside it replays the committed seed
//! corpus and every past finding through the same functions on every
//! `cargo test`, without nightly or `cargo-fuzz`. A panic is a finding:
//! the bar's release profile is `panic = "abort"`.

use super::format::{Civil, Format};
use super::tzif::{Abbr, Local, Tz};

#[cfg(test)]
mod tests;

/// Any bytes as a `--clock-format` (the first byte seeds a zone offset and
/// instants): parsing refuses or accepts without a panic, and an accepted
/// format renders any instant in any offset without a panic, without a
/// control character, and within a length bound.
pub fn format(data: &[u8]) {
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
    let instants = [
        i64::MIN,
        -1,
        0,
        1_790_651_220,
        i64::MAX,
        i64::from(seed) << 40,
    ];
    let mut out = String::new();
    for offset in offsets {
        let local = Local {
            offset,
            dst: seed & 1 == 1,
            abbr: Abbr::new(&data[..data.len().min(10)]),
        };
        for t in instants {
            out.clear();
            format.render(&Civil::at(t, local), &mut out);
            assert!(!out.chars().any(char::is_control), "{text:?} gave {out:?}");
            assert!(out.len() <= text.len() * 10 + 16, "{text:?} gave {out:?}");
        }
    }
}

/// Any bytes as a zone file and as a POSIX TZ string: refused or read
/// without a panic, and a zone read answers any instant with an offset
/// within a day and two hours of UTC and an abbreviation within its cap.
pub fn tzif(data: &[u8]) {
    if let Ok(tz) = Tz::parse(data) {
        probe(&tz);
    }
    if let Ok(tz) = Tz::posix(data) {
        probe(&tz);
    }
}

fn probe(tz: &Tz) {
    for t in [
        i64::MIN,
        -(1 << 40),
        -1,
        0,
        1_790_651_220,
        1 << 40,
        i64::MAX,
    ] {
        let local = tz.at(t);
        assert!(local.offset.abs() < 26 * 3600, "offset {}", local.offset);
        assert!(local.abbr.as_str().len() <= Abbr::CAP);
    }
}
