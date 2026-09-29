//! Any bytes as a zone file, and as a POSIX TZ string: refused or read
//! without a panic, and a zone read answers any instant, within a day and
//! two hours of UTC.

#![no_main]

include!("common.rs");

use tzif::Tz;

fn probe(tz: &Tz) {
    for t in [i64::MIN, -(1 << 40), -1, 0, 1_790_651_220, 1 << 40, i64::MAX] {
        let local = tz.at(t);
        assert!(local.offset.abs() < 26 * 3600, "offset {}", local.offset);
        assert!(local.abbr.as_str().len() <= tzif::Abbr::CAP);
    }
}

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(tz) = Tz::parse(data) {
        probe(&tz);
    }
    if let Ok(tz) = Tz::posix(data) {
        probe(&tz);
    }
});
