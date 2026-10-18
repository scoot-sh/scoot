//! The matcher's tests: folding, the mask contract, and scoring.

use super::*;

fn folded(text: &str) -> String {
    let mut out = String::new();
    fold_into(text, &mut out);
    out
}

fn scored(field: &str, query: &str) -> Option<i32> {
    score(field, query, &mut Dp::new())
}

#[test]
fn folding_lowercases_and_maps_sharp_s() {
    assert_eq!(folded("Firefox"), "firefox");
    assert_eq!(folded("Straße"), "strasse");
    assert_eq!(folded("STRASSE"), "strasse");
    assert_eq!(folded("ſerver"), "server");
    assert_eq!(folded("Kelvin"), "kelvin");
}

#[test]
fn folding_is_idempotent() {
    for text in [
        "Firefox",
        "Straße",
        "ẞ",
        "a-b_c/d.e",
        "Привет",
        "日本語",
        "İ",
    ] {
        let once = folded(text);
        assert_eq!(folded(&once), once, "{text}");
    }
}

#[test]
fn mask_subset_is_necessary_for_a_match() {
    let field = folded("Firefox Web Browser");
    let field_mask = mask_of(&field);
    // Every subsequence query passes the pre-filter.
    for query in ["firefox", "fx", "fbb", "web", "ser"] {
        let query_mask = mask_of(&folded(query));
        assert!(mask_allows(query_mask, field_mask), "{query}");
        assert!(scored(&field, &folded(query)).is_some(), "{query}");
    }
    // A query with an absent character never passes, never matches.
    for query in ["firefoxx", "qu", "zz"] {
        let query = folded(query);
        let query_mask = mask_of(&query);
        if !mask_allows(query_mask, field_mask) {
            assert!(scored(&field, &query).is_none(), "{query}");
        }
    }
}

#[test]
fn a_prefix_beats_a_scattered_match() {
    let prefix = scored(&folded("firefox"), &folded("fi")).unwrap();
    let scattered = scored(&folded("wifi config"), &folded("fi")).unwrap();
    assert!(prefix > scattered, "{prefix} vs {scattered}");
}

#[test]
fn consecutive_beats_gapped_and_starts_beat_middles() {
    let field = folded("text editor");
    let consecutive = scored(&field, &folded("te")).unwrap();
    let gapped = scored(&field, &folded("td")).unwrap();
    assert!(consecutive > gapped, "{consecutive} vs {gapped}");
    let at_start = scored(&field, &folded("tex")).unwrap();
    let mid_word = scored(&field, &folded("edi")).unwrap();
    assert!(at_start > mid_word, "{at_start} vs {mid_word}");
}

#[test]
fn word_starts_beat_mid_word_matches() {
    // "in" reaches the word start in "files inspector" (the later
    // `i`, not the mid-word one at index 1).
    let word_start = scored(&folded("files inspector"), &folded("in")).unwrap();
    let mid = scored(&folded("printer"), &folded("in")).unwrap();
    assert!(word_start > mid, "{word_start} vs {mid}");
}

#[test]
fn empty_query_matches_everything_with_zero() {
    assert_eq!(scored(&folded("firefox"), ""), Some(0));
    assert_eq!(scored("", ""), Some(0));
}

#[test]
fn a_non_subsequence_is_no_match() {
    assert_eq!(scored(&folded("firefox"), &folded("fxo")), None);
    assert_eq!(scored(&folded("ab"), &folded("abc")), None);
    assert_eq!(scored("", &folded("a")), None);
}

#[test]
fn one_char_queries_match() {
    assert!(scored(&folded("firefox"), &folded("f")).is_some());
    assert!(scored(&folded("firefox"), &folded("x")).is_some());
    assert_eq!(scored(&folded("firefox"), &folded("z")), None);
}

#[test]
fn unicode_matches_case_insensitively() {
    // "straße" folds to "strasse": the query matches across the
    // folding, in both cases.
    let field = folded("Berliner Straße");
    assert!(scored(&field, &folded("strasse")).is_some());
    assert!(scored(&field, &folded("STRASSE")).is_some());
    // Cyrillic lowercases through the same fold.
    let field = folded("Привет");
    assert!(scored(&field, &folded("привет")).is_some());
    assert!(scored(&field, &folded("ПРИВЕТ")).is_some());
}

#[test]
fn perfect_is_the_prefix_ceiling() {
    for (field, query) in [("firefox", "fir"), ("a", "a"), ("text editor", "text edit")] {
        let prefix = scored(&folded(field), &folded(query)).unwrap();
        let chars = folded(query).chars().count();
        assert_eq!(prefix, perfect(chars), "{field} / {query}");
    }
}

#[test]
fn hostile_bytes_never_panic() {
    let mut folded_text = String::new();
    for len in [0, 1, 2, 3, 7, 64] {
        let field: String = (0..len).map(|i| (i % 0x80) as u8 as char).collect();
        fold_into(&field, &mut folded_text);
        let mask = mask_of(&folded_text);
        let _ = scored(&folded_text, &folded_text);
        assert!(mask_allows(mask_of(&folded_text), mask));
    }
    // Unpaired surrogates cannot arise from &str; the longest
    // realistic blowup (every char expanding) still scores.
    let wide = "ẞ".repeat(512);
    let query = folded(&wide);
    let _ = scored(&query, &query);
}
