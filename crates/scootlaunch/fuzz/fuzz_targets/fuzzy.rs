//! Any bytes as a query: scoring refuses (no subsequence) or
//! accepts without a panic, and agreement with a plain subsequence
//! scan pins the dynamic program. The checks are `fuzz::fuzzy`
//! below.

#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../src/rank/fuzzy.rs"]
mod fuzzy;

mod fuzz {
    use super::fuzzy;

    fn is_subsequence(field: &str, query: &str) -> bool {
        let mut want = query.chars();
        let Some(mut next) = want.next() else {
            return true;
        };
        for char in field.chars() {
            if char == next {
                match want.next() {
                    Some(rest) => next = rest,
                    None => return true,
                }
            }
        }
        false
    }

    const FIELDS: &[&str] = &[
        "firefox",
        "firefox web browser",
        "berliner straße",
        "text editor",
        "system monitor",
        "a",
        "",
    ];

    pub fn fuzzy(data: &[u8]) -> Option<()> {
        let query = std::str::from_utf8(data).ok()?;
        let mut folded = String::new();
        fuzzy::fold_into(query, &mut folded);
        let mut dp = fuzzy::Dp::new();
        for field in FIELDS {
            let mut field_folded = String::new();
            fuzzy::fold_into(field, &mut field_folded);
            let scored = fuzzy::score(&field_folded, &folded, &mut dp);
            // The matcher's core contract: a score exists exactly for
            // subsequences, and a score implies the mask pre-filter
            // would have kept the field.
            assert_eq!(scored.is_some(), is_subsequence(&field_folded, &folded));
            if scored.is_some() {
                assert!(fuzzy::mask_allows(
                    fuzzy::mask_of(&folded),
                    fuzzy::mask_of(&field_folded)
                ));
            }
        }
        Some(())
    }
}

fuzz_target!(|data: &[u8]| {
    fuzz::fuzzy(data);
});
